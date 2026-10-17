use crate::jobs::{run_ticks_secs, JobManager};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use shared::{preset_by_id, DownloadJob, JobState, QualityKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

struct EngineCtx {
    http: reqwest::Client,
}

fn engine_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .expect("engine http client")
}

pub async fn run(mgr: Arc<JobManager>, id: String) {
    tracing::error!("ENGINE: run started");
    let result = run_inner(&mgr, &id).await;
    tracing::error!("ENGINE: run done");
    match result {
        Ok(path) => mgr.finish(&id, path),
        Err(e) => mgr.fail(&id, &e.to_string()),
    }
    mgr.task_exited(&id);
}

async fn run_inner(mgr: &Arc<JobManager>, id: &str) -> Result<PathBuf> {
    let (job, creds, is_audio, run_ticks, tmp_dir, final_name, downloads_dir, source_id, transcode) = {
        let entry = mgr.get_entry(id).ok_or_else(|| anyhow!("job vanished"))?;
        (
            entry.job,
            entry.creds,
            entry.is_audio,
            entry.run_ticks,
            entry.tmp_dir.clone(),
            entry.final_name.clone(),
            entry.downloads_dir.clone(),
            entry.source_id,
            entry.transcode,
        )
    };
    let preset = preset_by_id(&job.quality_id).ok_or_else(|| anyhow!("unknown quality preset"))?;
    let ctx = EngineCtx { http: engine_client() };

    let path = match preset.kind {
        QualityKind::Original => download_original(&ctx, mgr, &job, &creds, is_audio, source_id.as_deref(), &tmp_dir, &downloads_dir, &final_name).await?,
        QualityKind::Transcode => {
            download_transcode(&ctx, mgr, &job, &creds, run_ticks, &tmp_dir, &preset, source_id, transcode).await?;
            mgr.set_state(id, JobState::Remuxing);
            let final_path = remux(&job, &tmp_dir, &downloads_dir, &final_name).await?;
            let _ = std::fs::remove_dir_all(&tmp_dir);
            final_path
        }
    };
    Ok(path)
}

fn unique_final_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = name
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| name.to_string());
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_string());
    for i in 2..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}-{}.part", stem, uuid::Uuid::new_v4()))
}

fn part_file_path(tmp_dir: &Path) -> PathBuf {
    tmp_dir.join("file.part")
}

/// rename() can't cross filesystems (the downloads dir may be a bind mount),
/// so fall back to copy + delete when the kernel says EXDEV.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            std::fs::copy(from, to).map(|_| ())?;
            std::fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

fn auth_header(token: &str) -> String {
    format!(
        "MediaBrowser Client=\"Jellydown\", Device=\"Jellydown\", DeviceId=\"jellydown-desktop\", Version=\"0.1.0\", Token=\"{token}\""
    )
}

async fn finalize_original(
    mgr: &Arc<JobManager>,
    job_id: &str,
    tmp_dir: &Path,
    downloads_dir: &Path,
    final_name: &str,
    done: u64,
) -> Result<PathBuf> {
    let part = part_file_path(tmp_dir);
    if done == 0 {
        bail!("Nothing was downloaded");
    }
    let final_path = unique_final_path(downloads_dir, final_name);
    move_file(&part, &final_path).with_context(|| format!("moving file to {}", final_path.display()))?;
    mgr.set_bytes_total(job_id, done);
    Ok(final_path)
}

async fn download_original(
    ctx: &EngineCtx,
    mgr: &Arc<JobManager>,
    job: &DownloadJob,
    creds: &crate::jellyfin::Credentials,
    is_audio: bool,
    source_id: Option<&str>,
    tmp_dir: &Path,
    downloads_dir: &Path,
    final_name: &str,
) -> Result<PathBuf> {
    let url = crate::jellyfin::JellyfinClient::static_stream_url(creds, &job.item_id, is_audio, source_id);
    let part = part_file_path(tmp_dir);

    // The part file is the source of truth for how much we already have.
    let mut done: u64 = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if job.bytes_total == Some(done) && done > 0 {
        // Resumed after everything was already fetched.
        mgr.set_bytes_total(&job.id, done);
        return finalize_original(mgr, &job.id, tmp_dir, downloads_dir, final_name, done).await;
    }
    if job.bytes_total.is_some() && job.bytes_done != done {
        mgr.set_bytes_done(&job.id, done);
    }

    let mut attempts = 0;
    loop {
        attempts += 1;
        if attempts > 2 {
            bail!("Too many retries while talking to Jellyfin");
        }
        let mut req = ctx
            .http
            .get(&url)
            .header("Authorization", auth_header(&creds.token));
        if done > 0 {
            req = req.header("Range", format!("bytes={done}-"));
        }
        let resp = req.send().await.context("Could not reach Jellyfin stream endpoint")?;
        let status = resp.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            bail!("Item not found on the server (was it removed?)");
        }
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            if job.bytes_total == Some(done) && done > 0 {
                return finalize_original(mgr, &job.id, tmp_dir, downloads_dir, final_name, done).await;
            }
            bail!("Unexpected range response from Jellyfin");
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!("Jellyfin rejected the session token — log in again and restart this job");
        }
        let resp = resp.error_for_status()?;

        let content_length = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());

        if status == reqwest::StatusCode::OK && done > 0 {
            // Server ignored the Range header; start from scratch.
            done = 0;
            std::fs::File::create(&part)?.set_len(0)?;
            mgr.set_bytes_done(&job.id, 0);
        }
        if let Some(cl) = content_length {
            let total = if status == reqwest::StatusCode::PARTIAL_CONTENT { done + cl } else { cl };
            if total > 0 {
                mgr.set_bytes_total(&job.id, total);
            }
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part)
            .await
            .with_context(|| format!("opening {}", part.display()))?;

        let mut stream = resp.bytes_stream();
        let mut received: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            received += chunk.len() as u64;
            file.write_all(&chunk).await?;
            mgr.add_bytes(&job.id, chunk.len() as u64, None, None);
        }
        file.flush().await?;
        drop(file);

        let expected = mgr.get(&job.id).and_then(|e| e.job.bytes_total);
        if let Some(total) = expected {
            if done + received < total {
                // Connection closed before we got everything; resume on next attempt.
                done = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(done + received);
                debug!(job = %job.id, "stream ended early, will retry");
                continue;
            }
        }
        return finalize_original(mgr, &job.id, tmp_dir, downloads_dir, final_name, done + received).await;
    }
}

// ---------------- HLS / transcode ----------------

struct Playlist {
    /// (segment uri, duration seconds)
    segs: Vec<(String, f64)>,
    endlist: bool,
    /// fMP4 playlists carry an init segment (#EXT-X-MAP) with the moov box.
    init_uri: Option<String>,
}

fn parse_playlist(text: &str) -> Playlist {
    let mut segs = Vec::new();
    let mut endlist = false;
    let mut pending_dur: Option<f64> = None;
    let mut init_uri: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-ENDLIST") {
            endlist = true;
        } else if line.starts_with("#EXT-X-MAP:") {
            init_uri = line.split("URI=").nth(1).and_then(|u| {
                let u = u.trim().trim_matches('"');
                Some(u.to_string())
            });
        } else if line.starts_with("#EXTINF:") {
            let raw = line.trim_start_matches("#EXTINF:");
            let raw = raw.split(',').next().unwrap_or("0");
            pending_dur = raw.trim().parse::<f64>().ok();
        } else if line.starts_with('#') || line.is_empty() {
            continue;
        } else {
            segs.push((line.to_string(), pending_dur.take().unwrap_or(0.0)));
        }
    }
    Playlist { segs, endlist, init_uri }
}

fn segment_file(tmp_dir: &Path, index: usize) -> PathBuf {
    tmp_dir.join(format!("seg_{index:06}.ts"))
}

fn existing_segment_count(tmp_dir: &Path) -> usize {
    std::fs::read_dir(tmp_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with("seg_") && name.ends_with(".ts")
                })
                .count()
        })
        .unwrap_or(0)
}

/// URLs carry the api_key; keep it out of user-visible errors.
fn redact(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.query_pairs_mut().clear();
            u.set_query(None);
            u.to_string()
        }
        Err(_) => "<url>".to_string(),
    }
}

async fn fetch_text(ctx: &EngineCtx, url: &str, token: &str) -> Result<String> {
    let resp = ctx
        .http
        .get(url)
        .header("Authorization", auth_header(token))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .with_context(|| format!("requesting {}", redact(url)))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let short: String = body.chars().take(220).collect();
        bail!("Jellyfin returned HTTP {status} for {}: {short}", redact(url));
    }
    Ok(resp.text().await?)
}

async fn resolve_playlist(ctx: &EngineCtx, url: &str, token: &str) -> Result<(Playlist, String)> {
    let mut fetch_url = url.to_string();
    // master.m3u8 may contain a variant playlist; resolve at most twice.
    for _ in 0..2 {
        let text = fetch_text(ctx, &fetch_url, token).await?;
        if text.contains("#EXT-X-STREAM-INF") {
            let variant = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .ok_or_else(|| anyhow!("Malformed master playlist"))?;
            fetch_url = url::Url::parse(&fetch_url)?.join(variant)?.to_string();
            continue;
        }
        return Ok((parse_playlist(&text), fetch_url));
    }
    bail!("Could not resolve HLS media playlist");
}

async fn download_segment(
    ctx: &EngineCtx,
    token: &str,
    base_url: &str,
    uri: &str,
    dest: &Path,
) -> Result<u64> {
    let seg_url = url::Url::parse(base_url)?.join(uri)?.to_string();
    let part = dest.with_extension("ts.part");
    let resp = ctx
        .http
        .get(&seg_url)
        .header("Authorization", auth_header(token))
        .send()
        .await
        .with_context(|| format!("fetching segment {uri}"))?
        .error_for_status()?;
    let mut file = std::fs::File::create(&part)?;
    let mut bytes: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        bytes += chunk.len() as u64;
        file.write_all(&chunk)?;
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&part, dest)?;
    Ok(bytes)
}

async fn download_transcode(
    ctx: &EngineCtx,
    mgr: &Arc<JobManager>,
    job: &DownloadJob,
    creds: &crate::jellyfin::Credentials,
    run_ticks: Option<i64>,
    tmp_dir: &Path,
    preset: &shared::QualityPreset,
    source_id: Option<String>,
    transcode: Option<crate::jobs::TranscodeParams>,
) -> Result<()> {
    // Source-codec passthrough (e.g. HEVC -> HEVC); fall back to H.264 if the
    // server refuses to encode the requested codec.
    let (mut codec, mut bitrate) = match &transcode {
        Some(t) => (t.video_codec.clone(), t.video_bitrate),
        None => ("h264".to_string(), preset.video_bitrate.unwrap_or(4_000_000)),
    };
    let msid = transcode.as_ref().and_then(|t| t.media_source_id.clone()).or(source_id);
    let master_url = crate::jellyfin::JellyfinClient::hls_master_url(creds, &job.item_id, preset, &codec, bitrate, msid.as_deref());
    let mut done_count = existing_segment_count(tmp_dir);
    mgr.set_segments(&job.id, done_count as u32, None);

    let item_total_secs = run_ticks_secs(run_ticks);
    let mut media_url: Option<String> = None;
    let mut first_try = true;

    loop {
        let url = match &media_url {
            Some(u) => u.clone(),
            None => master_url.clone(),
        };
        tracing::info!(job = %job.id, segments = %done_count, "fetching HLS playlist");
        let (playlist, resolved_url) = match resolve_playlist(ctx, &url, &creds.token).await {
            Ok(r) => {
                tracing::info!(
                    job = %job.id,
                    listed = r.0.segs.len(),
                    endlist = r.0.endlist,
                    has_init = r.0.init_uri.is_some(),
                    "playlist resolved"
                );
                r
            }
            Err(e) if first_try => {
                warn!(job = %job.id, "transcode with {codec} failed ({e:#}); retrying with h264");
                codec = "h264".to_string();
                bitrate = preset.video_bitrate.unwrap_or(4_000_000);
                let fallback = crate::jellyfin::JellyfinClient::hls_master_url(creds, &job.item_id, preset, &codec, bitrate, msid.as_deref());
                resolve_playlist(ctx, &fallback, &creds.token).await?
            }
            Err(e) => return Err(e),
        };
        first_try = false;
        media_url = Some(resolved_url);

        let playlist_total: f64 = playlist.segs.iter().map(|(_, d)| *d).sum();
        let total_secs = item_total_secs.filter(|t| *t >= playlist_total * 0.8).unwrap_or(playlist_total);
        let total_segments = playlist.segs.len();

        if playlist.endlist {
            mgr.set_segments(&job.id, done_count as u32, Some(total_segments as u32));
        }

        // fMP4: grab the init segment (moov) once; remux needs it first in the concat list.
        tracing::info!(job = %job.id, "downloading init segment");
        if let Some(map_uri) = &playlist.init_uri {
            let init_dest = tmp_dir.join("init.mp4");
            if !init_dest.exists() {
                let base = media_url.as_deref().unwrap_or(&master_url);
                let init_part = tmp_dir.join("init.mp4.part");
                let seg_url = url::Url::parse(base)?.join(map_uri)?.to_string();
                let resp = ctx
                    .http
                    .get(&seg_url)
                    .header("Authorization", auth_header(&creds.token))
                    .send()
                    .await
                    .with_context(|| format!("fetching init segment {map_uri}"))?
                    .error_for_status()?;
                let mut f = std::fs::File::create(&init_part)?;
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    f.write_all(&chunk?)?;
                }
                f.flush()?;
                drop(f);
                std::fs::rename(&init_part, &init_dest)?;
            }
        }

        while done_count < total_segments {
            let (uri, _dur) = &playlist.segs[done_count];
            let dest = segment_file(tmp_dir, done_count);
            let t0 = std::time::Instant::now();
            tracing::info!(job = %job.id, seg = done_count, uri = %uri, "requesting segment");
            let bytes = download_segment(ctx, &creds.token, media_url.as_deref().unwrap_or(&master_url), uri, &dest)
                .await
                .with_context(|| format!("segment {done_count}"))?;
            tracing::info!(job = %job.id, seg = done_count, ms = t0.elapsed().as_millis() as u64, bytes, "segment done");
            done_count += 1;
            let downloaded_secs: f64 = playlist.segs[..done_count].iter().map(|(_, d)| *d).sum();
            mgr.set_segments(&job.id, done_count as u32, Some(total_segments as u32));
            mgr.add_bytes(&job.id, bytes, Some(downloaded_secs), Some(total_secs.max(0.001)));
        }

        if playlist.endlist {
            break;
        }
        // Playlist is still growing; give Jellyfin a moment to transcode more.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Ok(())
}

async fn remux(
    job: &DownloadJob,
    tmp_dir: &Path,
    downloads_dir: &Path,
    final_name: &str,
) -> Result<PathBuf> {
    let mut segs: Vec<PathBuf> = std::fs::read_dir(tmp_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            n.starts_with("seg_") && n.ends_with(".ts")
        })
        .collect();
    segs.sort();
    if segs.is_empty() {
        bail!("No segments were downloaded");
    }

    let concat_path = tmp_dir.join("concat.txt");
    {
        let mut f = std::fs::File::create(&concat_path)?;
        // fMP4 playlists ship an init segment that must come first.
        let init = tmp_dir.join("init.mp4");
        if init.exists() {
            writeln!(f, "file '{}'", init.canonicalize()?.to_string_lossy())?;
        }
        for s in &segs {
            writeln!(f, "file '{}'", s.canonicalize()?.to_string_lossy())?;
        }
    }
    // MPEG-TS carries ADTS AAC (needs aac_adtstoasc for mp4); fMP4 does not.
    let is_ts = segs
        .first()
        .and_then(|p| std::fs::File::open(p).ok())
        .and_then(|mut f| {
            use std::io::Read;
            let mut hdr = [0u8; 1];
            f.read_exact(&mut hdr).ok().map(|_| hdr[0] == 0x47)
        })
        .unwrap_or(false);
    let out_part = tmp_dir.join("remux.part");
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel", "error",
        "-f", "concat",
        "-safe", "0",
        "-i", concat_path.to_string_lossy().as_ref(),
        "-c", "copy",
        "-movflags", "+faststart",
        "-f", "mp4",
        out_part.to_string_lossy().as_ref(),
    ]);
    if is_ts {
        cmd.args(["-bsf:a", "aac_adtstoasc"]);
    }
    let output = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning ffmpeg (is ffmpeg installed?)")?;
    let output = output.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
        warn!(job = %job.id, "ffmpeg failed: {tail}");
        bail!("ffmpeg remux failed: {tail}");
    }
    let final_path = unique_final_path(downloads_dir, final_name);
    move_file(&out_part, &final_path)?;
    Ok(final_path)
}
