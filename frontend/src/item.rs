use crate::{api, format::*, icons::*, toast::push_toast, AppCtx};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use leptos::task::spawn_local;

/// Classify by the *larger* dimension — cinemascope 1080p is 1920×800, so
/// height alone mislabels it.
fn res_label(w: Option<u32>, h: Option<u32>) -> &'static str {
    let m = match (w, h) {
        (Some(w), Some(h)) => w.max(h),
        (Some(w), None) => w,
        (None, Some(h)) => h,
        (None, None) => 0,
    };
    match m {
        v if v >= 3400 => "4K",
        v if v >= 1800 => "1080p",
        v if v >= 1200 => "720p",
        v if v >= 640 => "480p",
        v if v > 0 => "SD",
        _ => "",
    }
}

fn media_line(d: &shared::ItemDetail) -> String {
    let mut parts = Vec::new();
    if let Some(m) = &d.media {
        if let Some(c) = &m.video_codec {
            parts.push(c.to_uppercase());
        }
        let res = res_label(m.width, m.height);
        if !res.is_empty() {
            parts.push(res.to_string());
        }
        if let Some(a) = &m.audio_codec {
            parts.push(a.to_uppercase());
        }
        if let Some(size) = m.size {
            parts.push(fmt_bytes(size));
        }
    }
    parts.join(" · ")
}

fn backdrop_for(d: &shared::ItemDetail) -> Option<String> {
    d.backdrop_url.clone().or_else(|| d.image_url.clone())
}

#[component]
pub fn ItemModal() -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));
    let detail = sctx.get_value().selected;
    let quality = RwSignal::new("original".to_string());
    let auto_save = RwSignal::new(true);
    let busy = RwSignal::new(false);

    view! {
        <Show when=move || detail.get().is_some()>
            {move || detail.get().map(|d| {
                let media = media_line(&d);
                let presets = shared::quality_presets();

                let close = move |_: leptos::ev::MouseEvent| detail.set(None);
                let close_btn = close.clone();

                let dl_id = d.id.clone();
                let dl_name = d.name.clone();
                let start_download = {
                    move |e: leptos::ev::MouseEvent| {
                        e.stop_propagation();
                        if busy.get_untracked() {
                            return;
                        }
                        busy.set(true);
                        let ctx = sctx.get_value();
                        let id = dl_id.clone();
                        let name = dl_name.clone();
                        let q = quality.get_untracked();
                        let save = auto_save.get_untracked();
                        spawn_local(async move {
                            match api::create_download(id, q, save).await {
                                Ok(_) => {
                                    push_toast(&ctx, "ok", format!("Added “{name}” to downloads"));
                                    detail.set(None);
                                    ctx.screen.set(crate::Screen::Downloads);
                                }
                                Err(msg) => push_toast(&ctx, "err", msg),
                            }
                            busy.set(false);
                        });
                    }
                };

                let backdrop_style = backdrop_for(&d)
                    .map(|u| format!("background-image:url('{u}')"))
                    .unwrap_or_default();

                view! {
                    <div class="overlay" on:click=close>
                        <div class="modal" on:click=|e| e.stop_propagation()>
                            <div class="modal-backdrop" style=backdrop_style>
                                <button class="icon-btn modal-close" on:click=close_btn>
                                    <IconX size=16/>
                                </button>
                            </div>
                            <div class="modal-body">
                                <h3 class="modal-title">{d.name.clone()}</h3>
                                <div class="modal-meta">
                                    {d.year.map(|y| view!{ <span>{y}</span> })}
                                    {(!d.item_type.is_empty()).then(|| view!{ <span>{d.item_type.clone()}</span> })}
                                    {(!media.is_empty()).then(|| view!{ <span class="mono">{media}</span> })}
                                </div>
                                {d.overview.clone().map(|o| view!{ <p class="modal-overview">{o}</p> })}

                                <div class="quality-label">"Quality"</div>
                                <p class="quality-note">"Bitrate scales with resolution: ≤1080p keep the source codec (HEVC stays HEVC); 4K sources encode to H.264 for speed."</p>
                                <div class="quality-list">
                                    {presets.iter().filter(|p| {
                                        // Tiers at or above the source resolution are pointless
                                        // re-encodes; the original already covers them.
                                        if p.kind != shared::QualityKind::Transcode {
                                            return true;
                                        }
                                        let src = d.media.as_ref()
                                            .and_then(|m| m.width.map(|w| w.max(m.height.unwrap_or(0))));
                                        let tgt = p.max_width.or(p.max_height);
                                        match (src, tgt) {
                                            (Some(s), Some(t)) => t < s,
                                            _ => true,
                                        }
                                    }).map(|p| {
                                        let id = p.id.clone();
                                        let selected_id = id.clone();
                                        let original_size = d.media.as_ref().and_then(|m| m.size);
                                        let est_text = if p.kind == shared::QualityKind::Original {
                                            original_size.map(fmt_bytes).unwrap_or_default()
                                        } else {
                                            estimate_size(p, d.media.as_ref(), d.run_time_ticks)
                                                .map(fmt_bytes)
                                                .map(|s| format!("≈ {s}"))
                                                .unwrap_or_default()
                                        };
                                        let bigger_than_original = p.kind == shared::QualityKind::Transcode
                                            && original_size
                                                .and_then(|o| estimate_size(p, d.media.as_ref(), d.run_time_ticks).map(|b| b > o))
                                                .unwrap_or(false);
                                        let est_final = if bigger_than_original {
                                            format!("{est_text} · bigger than original")
                                        } else {
                                            est_text
                                        };
                                        view! {
                                            <button
                                                class="quality-opt"
                                                class:selected=move || quality.get() == selected_id
                                                on:click=move |_| quality.set(id.clone())
                                            >
                                                <span class="quality-name">{p.label.clone()}</span>
                                                <span class="quality-est" class:warn=bigger_than_original>{est_final}</span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>

                                <div class="modal-actions">
                                    <label class="check">
                                        <input type="checkbox" prop:checked=move || auto_save.get() on:change=move |e| {
                                                    let checked = e
                                                        .target()
                                                        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                                        .map(|i| i.checked())
                                                        .unwrap_or(false);
                                                    auto_save.set(checked);
                                                }/>
                                        "Save to my device when done"
                                    </label>
                                    <button class="btn btn-primary" disabled=move || busy.get() on:click=start_download>
                                        <IconDownload size=15/>
                                        {move || if busy.get() { "Adding…" } else { "Download" }}
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                }
            })}
        </Show>
    }
}

#[component]
pub fn EpisodesModal() -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));
    let eps = sctx.get_value().episodes;
    let auto_save = RwSignal::new(true);
    let busy_id = RwSignal::new(None::<String>);

    view! {
        <Show when=move || eps.get().is_some()>
            {move || eps.get().map(|se| {
                let presets = shared::quality_presets();

                let close = move |_: leptos::ev::MouseEvent| eps.set(None);
                let close_btn = close.clone();

                let download_ep = {
                    move |e: leptos::ev::MouseEvent, ep_id: String, ep_name: String| {
                        e.stop_propagation();
                        if busy_id.get_untracked().is_some() {
                            return;
                        }
                        busy_id.set(Some(ep_id.clone()));
                        let ctx = sctx.get_value();
                        let q = ctx.episode_quality.get_untracked();
                        let save = auto_save.get_untracked();
                        spawn_local(async move {
                            match api::create_download(ep_id.clone(), q, save).await {
                                Ok(_) => push_toast(&ctx, "ok", format!("Added “{ep_name}” to downloads")),
                                Err(msg) => push_toast(&ctx, "err", msg),
                            }
                            busy_id.set(None);
                        });
                    }
                };

                view! {
                    <div class="overlay" on:click=close>
                        <div class="modal modal-episodes" on:click=|e| e.stop_propagation()>
                            <div class="modal-body">
                                <div class="modal-head-row">
                                    <h3 class="modal-title">{se.name.clone()}</h3>
                                    <button class="icon-btn" on:click=close_btn><IconX size=16/></button>
                                </div>

                                <div class="ep-quality-row">
                                    <span class="quality-label">"Download as"</span>
                                    <label class="check">
                                        <input type="checkbox" prop:checked=move || auto_save.get() on:change=move |e| {
                                                    let checked = e
                                                        .target()
                                                        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                                        .map(|i| i.checked())
                                                        .unwrap_or(false);
                                                    auto_save.set(checked);
                                                }/>
                                        "auto-save to device"
                                    </label>
                                    <select class="select"
                                        on:change=move |e| sctx.get_value().episode_quality.set(event_target_value(&e))
                                        prop:value=move || sctx.get_value().episode_quality.get()
                                    >
                                        {presets.iter().map(|p| {
                                            let id = p.id.clone();
                                            view! { <option value=id.clone()>{p.label.clone()}</option> }
                                        }).collect_view()}
                                    </select>
                                </div>

                                <div class="ep-list">
                                    <For
                                        each=move || se.list.clone()
                                        key=|ep| ep.id.clone()
                                        children=move |ep| {
                                            let download_ep = download_ep.clone();
                                            let ep_id_flag = ep.id.clone();
                                            view! {
                                                <div class="ep-row">
                                                    <span class="ep-code mono">
                                                        {format!("S{:02}E{:02}", ep.season.unwrap_or(0), ep.episode.unwrap_or(0))}
                                                    </span>
                                                    <span class="ep-name">{ep.name.clone()}</span>
                                                    <span class="ep-runtime muted">{fmt_runtime(ep.run_time_ticks)}</span>
                                                    <button
                                                        class="icon-btn"
                                                        title="Download episode"
                                                        disabled=move || busy_id.get().as_deref() == Some(ep_id_flag.as_str())
                                                        on:click=move |e| download_ep(e, ep.id.clone(), ep.name.clone())
                                                    >
                                                        <IconDownload size=15/>
                                                    </button>
                                                </div>
                                            }
                                        }
                                    />
                                </div>
                            </div>
                        </div>
                    </div>
                }
            })}
        </Show>
    }
}
