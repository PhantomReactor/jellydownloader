use crate::{api, format::*, icons::*, toast::push_toast, AppCtx};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared::{DownloadJob, JobState};

#[component]
pub fn DownloadsView() -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));
    let total_active = Memo::new(move |_| {
        sctx.get_value()
            .jobs
            .get()
            .iter()
            .filter(|j| matches!(j.state, JobState::Running | JobState::Remuxing | JobState::Queued))
            .count()
    });

    view! {
        <div class="page">
            <header class="page-head">
                <h2>"Downloads"</h2>
                <span class="muted">
                    {move || {
                        let n = total_active.get();
                        if n > 0 { format!("{n} active") } else { String::new() }
                    }}
                </span>
            </header>

            {move || {
                if sctx.get_value().jobs.get().is_empty() {
                    view! {
                        <div class="empty">
                            <IconDownload size=34/>
                            <p>"No downloads yet."</p>
                            <p class="muted">"Find something in Browse and pick a quality."</p>
                            <button class="btn" on:click=move |_| sctx.get_value().screen.set(crate::Screen::Library)>
                                "Go to Browse"
                            </button>
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <div class="dl-list">
                            <For
                                each=move || sctx.get_value().jobs.get()
                                key=|j| j.id.clone()
                                children=move |j| {
                                    let id = j.id.clone();
                                    view! { <JobRow job_id=id/> }
                                }
                            />
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

#[component]
fn JobRow(job_id: String) -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));
    let job = Memo::new(move |_| {
        sctx.get_value()
            .jobs
            .get()
            .into_iter()
            .find(|j| j.id == job_id)
    });
    let acting = RwSignal::new(false);

    view! {
        {move || job.get().map(|j| {
            let act = {
                let id_job = j.id.clone();
                move |e: leptos::ev::MouseEvent, action: &'static str| {
                    e.stop_propagation();
                    if acting.get_untracked() {
                        return;
                    }
                    acting.try_set(true);
                    let ctx = sctx.get_value();
                    let id = id_job.clone();
                    spawn_local(async move {
                        if let Err(msg) = api::job_action(id, action).await {
                            push_toast(&ctx, "err", msg);
                        }
                        acting.try_set(false);
                    });
                }
            };
            let act_pause = act.clone();
            let act_stop = act.clone();
            let act_start = act.clone();

            let remove = {
                let id_rm = j.id.clone();
                move |_: leptos::ev::MouseEvent| {
                    let ctx = sctx.get_value();
                    let id = id_rm.clone();
                    spawn_local(async move {
                        if let Err(msg) = api::remove_job(id, false).await {
                            push_toast(&ctx, "err", msg);
                        }
                    });
                }
            };

            let restart = {
                let id_rs = j.id.clone();
                move |e: leptos::ev::MouseEvent| {
                    e.stop_propagation();
                    let ctx = sctx.get_value();
                    let id = id_rs.clone();
                    spawn_local(async move {
                        if let Err(msg) = api::job_action(id, "restart").await {
                            push_toast(&ctx, "err", msg);
                        }
                    });
                }
            };

            let pct = j.progress.map(|p| format!("{:.0}%", (p * 100.0).min(100.0)));
            let pct_width = j
                .progress
                .map(|p| format!("{:.1}%", (p * 100.0).min(100.0)))
                .unwrap_or_else(|| "0%".into());
            let is_transcode = j.quality_id != "original";
            let size_text = if is_transcode {
                match j.segments_total {
                    Some(total) => format!("{} / {} segments", j.segments_done, total),
                    None => fmt_bytes(j.bytes_done),
                }
            } else {
                match j.bytes_total {
                    Some(total) => format!("{} / {}", fmt_bytes(j.bytes_done), fmt_bytes(total)),
                    None => fmt_bytes(j.bytes_done),
                }
            };
            let running = matches!(j.state, JobState::Running | JobState::Remuxing);
            let done = j.state == JobState::Completed;

            let buttons = match j.state {
                JobState::Running | JobState::Remuxing | JobState::Queued => {
                    view! {
                        <button class="icon-btn" title="Pause" on:click=move |e| act_pause(e, "pause")><IconPause size=15/></button>
                        <button class="icon-btn" title="Stop" on:click=move |e| act_stop(e, "stop")><IconStop size=15/></button>
                    }
                    .into_any()
                }
                JobState::Paused => view! {
                    <button class="icon-btn" title="Resume" on:click=move |e| act(e, "resume")><IconPlay size=15/></button>
                    <button class="icon-btn" title="Restart from scratch" on:click=restart><IconRestart size=15/></button>
                    <button class="icon-btn" title="Stop" on:click=move |e| act_stop(e, "stop")><IconStop size=15/></button>
                    <button class="icon-btn icon-danger" title="Remove" on:click=remove><IconTrash size=15/></button>
                }
                .into_any(),
                JobState::Stopped | JobState::Failed => view! {
                    <button class="icon-btn" title="Start" on:click=move |e| act_start(e, "start")><IconPlay size=15/></button>
                    <button class="icon-btn" title="Restart from scratch" on:click=restart><IconRestart size=15/></button>
                    <button class="icon-btn icon-danger" title="Remove" on:click=remove><IconTrash size=15/></button>
                }
                .into_any(),
                JobState::Completed => view! {
                    <a class="icon-btn" title="Save to device" href={format!("/api/downloads/{}/file", j.id)} download>
                        <IconDownload size=15/>
                    </a>
                    <button class="icon-btn" title="Download again" on:click=restart><IconRestart size=15/></button>
                    <button class="icon-btn icon-danger" title="Remove from list" on:click=remove><IconTrash size=15/></button>
                }
                .into_any(),
            };

            view! {
                <div class="dl-row" class:done=move || done>
                    <div class="dl-thumb">
                        {match j.image_url.clone() {
                            Some(u) => view! { <img src=u loading="lazy" onerror="this.style.display='none'"/> }.into_any(),
                            None => view! { <div class="poster-none"><IconFilm size=18/></div> }.into_any(),
                        }}
                    </div>
                    <div class="dl-main">
                        <div class="dl-top">
                            <span class="dl-name" title=j.item_name.clone()>{j.item_name.clone()}</span>
                            <span class="chip">{j.quality_label.clone()}</span>
                            <span class="chip state-chip" data-state=j.state.as_str()>{state_label(&j)}</span>
                        </div>
                        <div class="progress" class:running=running>
                            <div class="progress-fill" style=format!("width:{pct_width}")></div>
                        </div>
                        <div class="dl-stats mono">
                            <span>{pct.unwrap_or_else(|| "…".into())}</span>
                            <span>{size_text}</span>
                            <Show when=move || running>
                                <span>{fmt_speed(j.speed_bps)}</span>
                                {j.eta_seconds.map(|e| view! { <span>{format!("ETA {}", fmt_eta(Some(e)))}</span> })}
                            </Show>
                        </div>
                        {j.error.clone().map(|err| view! { <div class="dl-error">{err}</div> })}
                        {j.file_path.clone().map(|p| view! { <div class="dl-path mono">{p}</div> })}
                    </div>
                    <div class="dl-actions">{buttons}</div>
                </div>
            }
        })}
    }
}

/// The server-side work isn't "downloading" for transcodes — spell out what's
/// happening so users aren't confused about double transfers.
fn state_label(j: &DownloadJob) -> &'static str {
    match j.state {
        JobState::Queued => "Queued",
        JobState::Running => {
            if j.quality_id == "original" {
                "Copying from Jellyfin"
            } else {
                "Transcoding"
            }
        }
        JobState::Paused => "Paused",
        JobState::Stopped => "Stopped",
        JobState::Remuxing => "Packaging",
        JobState::Completed => "Done",
        JobState::Failed => "Failed",
    }
}
