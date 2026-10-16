use crate::config::Config;
use crate::engine;
use crate::jellyfin::{Credentials, JellyfinClient};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::{
    DownloadJob, ItemDetail, JobState, QualityPreset,
};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::{info, warn};

struct JobRuntime {
    creds: Credentials,
    /// Session that created the job (for auto-save delivery).
    owner_session: Option<String>,
    is_audio: bool,
    run_ticks: Option<i64>,
    container_ext: Option<String>,
    abort: Option<AbortHandle>,
    /// (time, cumulative bytes at that time) ring buffer for speed estimation.
    samples: VecDeque<(Instant, u64)>,
    last_persist: Option<Instant>,
    tmp_dir: PathBuf,
    final_name: String,
    downloads_dir: PathBuf,
}

struct JobEntry {
    job: DownloadJob,
    rt: JobRuntime,
}

#[derive(Serialize, Deserialize)]
struct PersistedJob {
    job: DownloadJob,
    creds: PersistedCreds,
    is_audio: bool,
    run_ticks: Option<i64>,
    container_ext: Option<String>,
    final_name: String,
    #[serde(default)]
    owner_session: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PersistedCreds {
    server_url: String,
    user_id: String,
    token: String,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedState {
    jobs: Vec<PersistedJob>,
}

struct JobsInner {
    jobs: HashMap<String, JobEntry>,
    order: Vec<String>,
}

pub struct JobManager {
    inner: Mutex<JobsInner>,
    pub config: Arc<Config>,
    pub client: Arc<JellyfinClient>,
    state_path: PathBuf,
    wake: mpsc::UnboundedSender<()>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let keep = !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && (ch as u32) >= 32;
        if keep {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim().trim_matches('.').trim();
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "download".to_string()
    } else {
        collapsed
    }
}

impl JobManager {
    pub fn new(config: Arc<Config>, client: Arc<JellyfinClient>) -> Arc<Self> {
        std::fs::create_dir_all(&config.data_dir).ok();
        std::fs::create_dir_all(&config.downloads_dir).ok();
        let state_path = config.data_dir.join("state.json");
        let mut jobs = HashMap::new();
        let mut order = Vec::new();
        if let Ok(raw) = std::fs::read_to_string(&state_path) {
            if let Ok(state) = serde_json::from_str::<PersistedState>(&raw) {
                for p in state.jobs {
                    let mut job = p.job;
                    // Anything that claimed to be running died with the process.
                    if job.state == JobState::Running || job.state == JobState::Remuxing {
                        job.state = JobState::Paused;
                    }
                    let tmp_dir = tmp_dir_for(&config, &job.id);
                    let entry = JobEntry {
                        job: job.clone(),
                        rt: JobRuntime {
                            owner_session: p.owner_session,
                            creds: Credentials {
                                server_url: p.creds.server_url,
                                user_id: p.creds.user_id,
                                token: p.creds.token,
                            },
                            is_audio: p.is_audio,
                            run_ticks: p.run_ticks,
                            container_ext: p.container_ext,
                            abort: None,
                            samples: VecDeque::new(),
                            last_persist: None,
                            tmp_dir,
                            final_name: p.final_name,
                            downloads_dir: config.downloads_dir.clone(),
                        },
                    };
                    order.push(entry.job.id.clone());
                    jobs.insert(entry.job.id.clone(), entry);
                }
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let mgr = Arc::new(Self {
            inner: Mutex::new(JobsInner { jobs, order }),
            config,
            client,
            state_path,
            wake: tx,
        });
        tokio::spawn(scheduler_loop(Arc::clone(&mgr), rx));
        mgr
    }

    fn persist(&self, inner: &JobsInner) {
        let state = PersistedState {
            jobs: inner
                .order
                .iter()
                .filter_map(|id| inner.jobs.get(id))
                .map(|e| PersistedJob {
                    job: e.job.clone(),
                    creds: PersistedCreds {
                        server_url: e.rt.creds.server_url.clone(),
                        user_id: e.rt.creds.user_id.clone(),
                        token: e.rt.creds.token.clone(),
                    },
                    is_audio: e.rt.is_audio,
                    run_ticks: e.rt.run_ticks,
                    container_ext: e.rt.container_ext.clone(),
                    final_name: e.rt.final_name.clone(),
                    owner_session: e.rt.owner_session.clone(),
                })
                .collect(),
        };
        if let Ok(raw) = serde_json::to_string(&state) {
            let tmp = self.state_path.with_extension("tmp");
            if std::fs::write(&tmp, raw).is_ok() {
                let _ = std::fs::rename(&tmp, &self.state_path);
            }
        }
    }

    fn persist_throttled(&self, inner: &mut JobsInner, id: &str) {
        if let Some(e) = inner.jobs.get_mut(id) {
            let now = Instant::now();
            let due = e.rt.last_persist.map(|t| now.duration_since(t) > Duration::from_millis(500)).unwrap_or(true);
            if due {
                e.rt.last_persist = Some(now);
                self.persist(inner);
            }
        }
    }

    fn count_running(inner: &JobsInner) -> usize {
        inner
            .jobs
            .values()
            .filter(|e| e.job.state == JobState::Running || e.job.state == JobState::Remuxing)
            .count()
    }

    /// Start the first queued job if there is a free slot. Repeat until no slot/free job.
    fn try_start_next(self: &Arc<Self>) {
        let max = self.config.parallel_downloads.max(1) as usize;
        loop {
            let to_start = {
                let mut inner = self.inner.lock().unwrap();
                let free = max.saturating_sub(Self::count_running(&inner));
                if free == 0 {
                    break;
                }
                let id = inner
                    .order
                    .iter()
                    .find(|id| inner.jobs.get(*id).map(|e| e.job.state == JobState::Queued).unwrap_or(false))
                    .cloned();
                match id {
                    Some(id) => {
                        if let Some(e) = inner.jobs.get_mut(&id) {
                            e.job.state = JobState::Running;
                            e.job.error = None;
                        }
                        Some(id)
                    }
                    None => break,
                }
            };
            let Some(id) = to_start else { break };
            self.spawn_engine(id);
        }
    }

    fn spawn_engine(self: &Arc<Self>, id: String) {
        let mgr = Arc::clone(self);
        let id_for_task = id.clone();
        let handle = tokio::spawn(async move {
            engine::run(mgr, id_for_task).await;
        });
        let abort = handle.abort_handle();
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(&id) {
            e.rt.abort = Some(abort);
        }
    }

    pub fn create(
        self: &Arc<Self>,
        detail: ItemDetail,
        preset: QualityPreset,
        creds: Credentials,
        auto_save: bool,
        owner_session: Option<String>,
    ) -> Result<DownloadJob> {
        if detail.item_type == "Series" {
            anyhow::bail!("Select an episode to download, not the whole series");
        }
        if detail.item_type == "Season" {
            anyhow::bail!("Select an episode to download, not the whole season");
        }
        let is_audio = detail.item_type == "Audio";
        let id = uuid::Uuid::new_v4().to_string();

        let display = display_name(&detail);
        let ext = match preset.kind {
            shared::QualityKind::Original => {
                let raw = detail
                    .media
                    .as_ref()
                    .and_then(|m| m.container.clone())
                    .unwrap_or_else(|| "mp4".to_string());
                let raw = raw.split(',').next().unwrap_or("mp4").to_string();
                raw
            }
            shared::QualityKind::Transcode => "mp4".to_string(),
        };
        let final_name = match preset.kind {
            shared::QualityKind::Original => format!("{display}.{ext}"),
            shared::QualityKind::Transcode => format!("{display} [{}].mp4", preset.id),
        };

        let tmp_dir = tmp_dir_for(&self.config, &id);
        std::fs::create_dir_all(&tmp_dir)?;

        let job = DownloadJob {
            id: id.clone(),
            item_id: detail.id.clone(),
            item_name: display,
            series_name: detail.series_name.clone(),
            year: detail.year,
            image_url: detail.image_url.clone(),
            quality_id: preset.id.to_string(),
            quality_label: preset.label.to_string(),
            progress: None,
            state: JobState::Queued,
            bytes_done: 0,
            bytes_total: detail.media.as_ref().and_then(|m| m.size).filter(|_| preset.kind == shared::QualityKind::Original),
            segments_done: 0,
            segments_total: None,
            speed_bps: 0.0,
            eta_seconds: None,
            error: None,
            file_path: None,
            created_at: now_secs(),
            auto_save,
        };

        let mut inner = self.inner.lock().unwrap();
        let entry = JobEntry {
            job,
            rt: JobRuntime {
                owner_session,
                creds,
                is_audio,
                run_ticks: detail.run_time_ticks,
                container_ext: detail.media.as_ref().and_then(|m| m.container.clone()),
                abort: None,
                samples: VecDeque::new(),
                last_persist: None,
                tmp_dir,
                final_name,
                downloads_dir: self.config.downloads_dir.clone(),
            },
        };
        inner.order.push(id.clone());
        inner.jobs.insert(id.clone(), entry);
        self.persist(&inner);
        drop(inner);
        self.wake.send(()).ok();
        let job = self.get(&id).expect("just inserted").job;
        Ok(job)
    }

    pub fn get(&self, id: &str) -> Option<JobEntryClone> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.get(id).map(|e| JobEntryClone {
            job: e.job.clone(),
        })
    }

    /// Snapshot of everything the engine task needs to (re)start a job.
    pub fn get_entry(&self, id: &str) -> Option<EntrySnapshot> {
        let inner = self.inner.lock().unwrap();
        inner.jobs.get(id).map(|e| EntrySnapshot {
            job: e.job.clone(),
            creds: e.rt.creds.clone(),
            is_audio: e.rt.is_audio,
            run_ticks: e.rt.run_ticks,
            tmp_dir: e.rt.tmp_dir.clone(),
            final_name: e.rt.final_name.clone(),
            downloads_dir: e.rt.downloads_dir.clone(),
        })
    }

    pub fn set_bytes_total(&self, id: &str, total: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.job.bytes_total = Some(total);
            self.persist_throttled(&mut inner, id);
        }
    }

    pub fn set_bytes_done(&self, id: &str, done: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.job.bytes_done = done;
            e.job.progress = e.job.bytes_total.and_then(|t| {
                if t > 0 {
                    Some((done as f64 / t as f64).clamp(0.0, 0.999))
                } else {
                    None
                }
            });
            self.persist_throttled(&mut inner, id);
        }
    }

    pub fn list(&self) -> Vec<DownloadJob> {
        let inner = self.inner.lock().unwrap();
        inner
            .order
            .iter()
            .rev()
            .filter_map(|id| inner.jobs.get(id))
            .map(|e| e.job.clone())
            .collect()
    }

    /// Completed jobs with auto_save enabled that belong to `session_id`.
    pub fn list_save_ready(&self, session_id: &str) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .order
            .iter()
            .rev()
            .filter_map(|id| inner.jobs.get(id))
            .filter(|e| {
                e.job.state == JobState::Completed
                    && e.job.auto_save
                    && e.rt.owner_session.as_deref() == Some(session_id)
            })
            .map(|e| e.job.id.clone())
            .collect()
    }

    pub fn pause(&self, id: &str) -> Result<()> {
        let abort = {
            let mut inner = self.inner.lock().unwrap();
            let e = inner.jobs.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            if !matches!(e.job.state, JobState::Running | JobState::Remuxing | JobState::Queued) {
                anyhow::bail!("Job cannot be paused in state '{}'", e.job.state.as_str());
            }
            let abort = e.rt.abort.take();
            e.job.state = JobState::Paused;
            self.persist(&inner);
            abort
        };
        if let Some(a) = abort {
            a.abort();
        }
        self.wake.send(()).ok();
        Ok(())
    }

    /// Resume a paused/stopped/failed job, or start a queued one.
    pub fn start(&self, id: &str) -> Result<()> {
        {
            let mut inner = self.inner.lock().unwrap();
            let e = inner.jobs.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            if !matches!(e.job.state, JobState::Paused | JobState::Stopped | JobState::Failed | JobState::Queued) {
                anyhow::bail!("Job cannot start in state '{}'", e.job.state.as_str());
            }
            e.job.state = JobState::Queued;
            e.job.error = None;
            self.persist(&inner);
        }
        self.wake.send(()).ok();
        Ok(())
    }

    pub fn stop(&self, id: &str) -> Result<()> {
        let abort = {
            let mut inner = self.inner.lock().unwrap();
            let e = inner.jobs.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            if matches!(e.job.state, JobState::Completed) {
                anyhow::bail!("Job already completed");
            }
            if matches!(e.job.state, JobState::Stopped) {
                anyhow::bail!("Job already stopped");
            }
            let abort = e.rt.abort.take();
            e.job.state = JobState::Stopped;
            self.persist(&inner);
            abort
        };
        if let Some(a) = abort {
            a.abort();
        }
        self.wake.send(()).ok();
        Ok(())
    }

    pub fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        {
            let mut inner = self.inner.lock().unwrap();
            let e = inner.jobs.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            if matches!(e.job.state, JobState::Running | JobState::Remuxing | JobState::Queued) {
                anyhow::bail!("Stop the job before restarting it");
            }
            let abort = e.rt.abort.take();
            // wipe partial data and start over
            let _ = std::fs::remove_dir_all(&e.rt.tmp_dir);
            std::fs::create_dir_all(&e.rt.tmp_dir).ok();
            let final_path = e.rt.downloads_dir.join(&e.rt.final_name);
            if e.job.state == JobState::Completed && final_path.exists() {
                let _ = std::fs::remove_file(&final_path);
            }
            e.job.state = JobState::Queued;
            e.job.bytes_done = 0;
            e.job.bytes_total = None;
            e.job.segments_done = 0;
            e.job.segments_total = None;
            e.job.progress = None;
            e.job.speed_bps = 0.0;
            e.job.eta_seconds = None;
            e.job.error = None;
            e.job.file_path = None;
            self.persist(&inner);
            if let Some(a) = abort {
                a.abort();
            }
        }
        self.wake.send(()).ok();
        Ok(())
    }

    pub fn remove(self: &Arc<Self>, id: &str, delete_file: bool) -> Result<()> {
        let (abort, tmp_dir, final_path) = {
            let mut inner = self.inner.lock().unwrap();
            let e = inner.jobs.get_mut(id).ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            let abort = e.rt.abort.take();
            let tmp_dir = e.rt.tmp_dir.clone();
            let final_path = e.rt.downloads_dir.join(&e.rt.final_name);
            inner.jobs.remove(id);
            inner.order.retain(|x| x != id);
            self.persist(&inner);
            (abort, tmp_dir, final_path)
        };
        if let Some(a) = abort {
            a.abort();
        }
        let _ = std::fs::remove_dir_all(tmp_dir);
        if delete_file && final_path.exists() {
            let _ = std::fs::remove_file(final_path);
        }
        self.wake.send(()).ok();
        Ok(())
    }

    // ---- engine callbacks ----

    pub fn add_bytes(&self, id: &str, n: u64, downloaded_secs: Option<f64>, total_secs: Option<f64>) {
        let mut inner = self.inner.lock().unwrap();
        let Some(e) = inner.jobs.get_mut(id) else { return };
        e.job.bytes_done += n;
        let now = Instant::now();
        let samples = &mut e.rt.samples;
        samples.push_back((now, e.job.bytes_done));
        while samples.len() > 10 {
            samples.pop_front();
        }
        if samples.len() >= 2 {
            let (t0, b0) = *samples.front().unwrap();
            let (t1, b1) = *samples.back().unwrap();
            let dt = t1.duration_since(t0).as_secs_f64();
            if dt > 0.2 {
                let inst = (b1 - b0) as f64 / dt;
                e.job.speed_bps = if e.job.speed_bps == 0.0 { inst } else { e.job.speed_bps * 0.6 + inst * 0.4 };
            }
        }
        // progress + eta
        match total_secs {
            Some(total) if total > 0.0 => {
                let secs = downloaded_secs.unwrap_or(0.0);
                e.job.progress = Some((secs / total).clamp(0.0, 0.999));
                let speed = e.job.speed_bps;
                if speed > 0.0 {
                    let done_secs = downloaded_secs.unwrap_or(0.0);
                    let bps_per_sec = if done_secs > 0.0 { e.job.bytes_done as f64 / done_secs } else { 0.0 };
                    let remaining_bytes = (total - done_secs) * bps_per_sec;
                    e.job.eta_seconds = Some((remaining_bytes / speed) as u64);
                }
            }
            Some(_) => {}
            None => {
                if let Some(total_bytes) = e.job.bytes_total {
                    if total_bytes > 0 {
                        e.job.progress = Some((e.job.bytes_done as f64 / total_bytes as f64).clamp(0.0, 0.999));
                        if e.job.speed_bps > 0.0 {
                            e.job.eta_seconds = Some(((total_bytes - e.job.bytes_done) as f64 / e.job.speed_bps) as u64);
                        }
                    }
                }
            }
        }
        self.persist_throttled(&mut inner, id);
    }

    pub fn set_segments(&self, id: &str, done: u32, total: Option<u32>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.job.segments_done = done;
            e.job.segments_total = total;
            self.persist_throttled(&mut inner, id);
        }
    }

    pub fn set_state(&self, id: &str, state: JobState) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.job.state = state;
            self.persist(&inner);
        }
    }

    pub fn finish(&self, id: &str, final_path: PathBuf) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.job.state = JobState::Completed;
            e.job.progress = Some(1.0);
            e.job.speed_bps = 0.0;
            e.job.eta_seconds = None;
            e.job.file_path = Some(final_path.to_string_lossy().into_owned());
            if e.job.bytes_total.is_none() {
                e.job.bytes_total = Some(e.job.bytes_done);
            }
            e.rt.abort = None;
            self.persist(&inner);
        }
        info!(job = id, "download completed");
    }

    pub fn fail(&self, id: &str, msg: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            // Don't overwrite pause/stop states set by the controller while aborting.
            if matches!(e.job.state, JobState::Paused | JobState::Stopped) {
                return;
            }
            e.job.state = JobState::Failed;
            e.job.error = Some(msg.to_string());
            e.job.speed_bps = 0.0;
            e.job.eta_seconds = None;
            e.rt.abort = None;
            self.persist(&inner);
        }
        warn!(job = id, "download failed: {msg}");
    }

    /// Called when an engine task exits for any reason: clears runtime bookkeeping and frees a slot.
    pub fn task_exited(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(e) = inner.jobs.get_mut(id) {
            e.rt.abort = None;
            e.rt.samples.clear();
        }
        drop(inner);
        self.wake.send(()).ok();
    }
}

async fn scheduler_loop(mgr: Arc<JobManager>, mut rx: mpsc::UnboundedReceiver<()>) {
    // Initial kick (restored queued jobs).
    mgr.try_start_next();
    while rx.recv().await.is_some() {
        mgr.try_start_next();
    }
}

fn tmp_dir_for(config: &Config, job_id: &str) -> PathBuf {
    config.data_dir.join("tmp").join(job_id)
}

pub struct JobEntryClone {
    pub job: DownloadJob,
}

pub struct EntrySnapshot {
    pub job: DownloadJob,
    pub creds: Credentials,
    pub is_audio: bool,
    pub run_ticks: Option<i64>,
    pub tmp_dir: PathBuf,
    pub final_name: String,
    pub downloads_dir: PathBuf,
}

/// Human-friendly file base name for an item.
pub fn display_name(detail: &ItemDetail) -> String {
    let base = match (&detail.series_name, detail.season, detail.episode) {
        (Some(series), Some(s), Some(e)) => format!("{series} S{s:02}E{e:02} - {}", detail.name),
        _ => detail.name.clone(),
    };
    match detail.year {
        Some(y) => format!("{sanitize} ({y})", sanitize = sanitize_filename(&base)),
        None => sanitize_filename(&base),
    }
}

pub fn run_ticks_secs(ticks: Option<i64>) -> Option<f64> {
    ticks.map(|t| t as f64 / 10_000_000.0)
}
