mod api;
mod downloads;
mod format;
mod icons;
mod item;
mod library;
mod login;
mod toast;

use downloads::DownloadsView;
use icons::{IconDownload, IconFilm, IconLogout};
use item::{EpisodesModal, ItemModal};
use leptos::prelude::*;
use leptos::task::spawn_local;
use library::LibraryView;
use login::LoginPage;
use shared::{DownloadJob, User};
use std::sync::atomic::{AtomicU64, Ordering};
use toast::{push_toast, Toast, Toasts};

#[derive(Clone, PartialEq)]
pub enum Screen {
    Library,
    Downloads,
}

#[derive(Clone)]
pub struct SeriesEpisodes {
    pub name: String,
    pub list: Vec<shared::ItemSummary>,
}

#[derive(Clone)]
pub struct AppCtx {
    pub user: RwSignal<Option<User>>,
    pub screen: RwSignal<Screen>,
    pub jobs: RwSignal<Vec<DownloadJob>>,
    pub toasts: RwSignal<Vec<Toast>>,
    pub selected: RwSignal<Option<shared::ItemDetail>>,
    pub episodes: RwSignal<Option<SeriesEpisodes>>,
    pub active_library: RwSignal<Option<String>>,
    pub episode_quality: RwSignal<String>,
}

static TOAST_ID: AtomicU64 = AtomicU64::new(0);

const SAVED_KEY: &str = "jd.saved";

fn saved_ids() -> Vec<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SAVED_KEY).ok().flatten())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn was_saved(id: &str) -> bool {
    saved_ids().iter().any(|x| x == id)
}

fn mark_saved(id: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let mut ids = saved_ids();
        ids.push(id.to_string());
        if let Ok(raw) = serde_json::to_string(&ids) {
            let _ = storage.set_item(SAVED_KEY, &raw);
        }
    }
}

/// Clicks a hidden anchor so the browser saves the file natively.
fn trigger_browser_download(id: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let Ok(el) = doc.create_element("a") else { return };
    let Ok(a) = el.dyn_into::<web_sys::HtmlAnchorElement>() else { return };
    let _ = a.set_href(&format!("/api/downloads/{id}/file"));
    let _ = a.set_attribute("download", "");
    if let Some(body) = doc.body() {
        let _ = body.append_child(&a);
        a.click();
        let _ = a.remove();
    }
}

pub fn next_toast_id() -> u64 {
    TOAST_ID.fetch_add(1, Ordering::Relaxed) + 1
}

use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsCast;

#[wasm_bindgen(start)]
fn run() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let user = RwSignal::new(None);
    let screen = RwSignal::new(Screen::Library);
    let jobs = RwSignal::new(Vec::new());
    let toasts = RwSignal::new(Vec::<Toast>::new());
    let ctx = AppCtx {
        user,
        screen,
        jobs,
        toasts,
        selected: RwSignal::new(None),
        episodes: RwSignal::new(None),
        active_library: RwSignal::new(None),
        episode_quality: RwSignal::new("original".to_string()),
    };
    provide_context(ctx);

    // Restore an existing session (cookie) on page load.
    spawn_local(async move {
        if let Ok(u) = api::me().await {
            user.set(Some(u));
        }
    });

    // Keep the download list fresh while signed in, and offer finished
    // auto-save jobs to the browser as soon as they appear.
    spawn_local(async move {
        loop {
            if user.get_untracked().is_some() {
                if let Ok(list) = api::jobs().await {
                    for id in &list.save_ready {
                        if !was_saved(id) {
                            mark_saved(id);
                            trigger_browser_download(id);
                        }
                    }
                    jobs.set(list.jobs);
                }
            }
            gloo_timers::future::TimeoutFuture::new(800).await;
        }
    });

    view! {
        <Toasts/>
        <Show when=move || user.get().is_some() fallback=move || view! { <LoginPage/> }>
            <Shell/>
        </Show>
    }
}

#[component]
fn Shell() -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));

    let host = Memo::new(move |_| {
        sctx.get_value()
            .user
            .get()
            .as_ref()
            .map(|u| {
                u.server_url
                    .trim_start_matches("http://")
                    .trim_start_matches("https://")
                    .to_string()
            })
            .unwrap_or_default()
    });

    let active_count = Memo::new(move |_| {
        sctx.get_value()
            .jobs
            .get()
            .iter()
            .filter(|j| matches!(j.state, shared::JobState::Running | shared::JobState::Remuxing | shared::JobState::Queued))
            .count()
    });

    let logout = move |_: leptos::ev::MouseEvent| {
        let ctx = sctx.get_value();
        spawn_local(async move {
            let _ = api::logout().await;
            ctx.user.set(None);
            ctx.jobs.set(Vec::new());
            ctx.selected.set(None);
            ctx.episodes.set(None);
            ctx.screen.set(Screen::Library);
            push_toast(&ctx, "ok", "Signed out");
        });
    };

    let nav_library = {
        let sctx = sctx;
        move |_: leptos::ev::MouseEvent| sctx.get_value().screen.set(Screen::Library)
    };
    let nav_downloads = move |_: leptos::ev::MouseEvent| sctx.get_value().screen.set(Screen::Downloads);

    view! {
        <div class="shell">
            <aside class="sidebar">
                <div class="brand">
                    <span class="brand-mark"></span>
                    <span class="brand-name">"jellydown"</span>
                </div>
                <nav class="nav">
                    <button
                        class="nav-item"
                        class:active=move || sctx.get_value().screen.get() == Screen::Library
                        on:click=nav_library
                    >
                        <IconFilm size=17/>
                        <span>"Browse"</span>
                    </button>
                    <button
                        class="nav-item"
                        class:active=move || sctx.get_value().screen.get() == Screen::Downloads
                        on:click=nav_downloads
                    >
                        <IconDownload size=17/>
                        <span>"Downloads"</span>
                        <Show when=move || (active_count.get() > 0)>
                            <span class="badge">{move || active_count.get()}</span>
                        </Show>
                    </button>
                </nav>
                <div class="side-foot">
                    <div class="side-user">
                        <span class="side-name">{move || sctx.get_value().user.get().map(|u| u.name).unwrap_or_default()}</span>
                        <span class="side-host mono">{move || host.get()}</span>
                    </div>
                    <button class="icon-btn" title="Sign out" on:click=logout>
                        <IconLogout size=15/>
                    </button>
                </div>
            </aside>
            <main class="main">
                <Show when=move || sctx.get_value().screen.get() == Screen::Library fallback=move || view! { <DownloadsView/> }>
                    <LibraryView/>
                </Show>
            </main>
        </div>
        <ItemModal/>
        <EpisodesModal/>
    }
}
