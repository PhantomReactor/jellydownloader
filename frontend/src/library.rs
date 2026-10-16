use crate::{api, icons::*, toast::push_toast, AppCtx};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn LibraryView() -> impl IntoView {
    let sctx = StoredValue::new(use_context::<AppCtx>().expect("app context"));
    let (libs, set_libs) = signal(Vec::<shared::LibraryView>::new());
    let search_input = RwSignal::new(String::new());
    let list = RwSignal::new(Vec::<shared::ItemSummary>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let reload = RwSignal::new(0u64);

    spawn_local(async move {
        if let Ok(v) = api::libraries().await {
            set_libs.set(v);
        }
    });

    // Debounced search: commit 300ms after the last keystroke.
    Effect::new(move |_| {
        let typed = search_input.get();
        spawn_local(async move {
            TimeoutFuture::new(300).await;
            // The component may be gone by the time this lands.
            if search_input.try_get_untracked().as_deref() == Some(typed.as_str()) {
                reload.try_update(|v| *v += 1);
            }
        });
    });

    Effect::new(move |_| {
        let _token = reload.get();
        let lib = sctx.get_value().active_library.get_untracked();
        let term = search_input.get_untracked();
        spawn_local(async move {
            loading.try_set(true);
            error.try_set(None);
            let term = if term.trim().is_empty() { None } else { Some(term) };
            match api::items(lib, term).await {
                Ok(v) => {
                    list.try_set(v);
                }
                Err(e) => {
                    error.try_set(Some(e));
                }
            }
            loading.try_set(false);
        });
    });

    view! {
        <div class="page">
            <header class="page-head">
                <h2>"Browse"</h2>
                <div class="search-box">
                    <IconSearch size=15/>
                    <input
                        type="text"
                        placeholder="Search movies, shows, episodes…"
                        prop:value=move || search_input.get()
                        on:input=move |e| search_input.set(event_target_value(&e))
                    />
                </div>
            </header>

            <div class="chip-row">
                <button
                    class="lib-chip"
                    class:active=move || sctx.get_value().active_library.get().is_none()
                    on:click=move |_| sctx.get_value().active_library.set(None)
                >"All"</button>
                <For
                    each=move || libs.get()
                    key=|l| l.id.clone()
                    children=move |l| {
                        let active_lib = sctx.get_value().active_library;
                        let id = l.id.clone();
                        view! {
                            <button
                                class="lib-chip"
                                class:active=move || active_lib.get().as_deref() == Some(id.as_str())
                                on:click=move |_| active_lib.set(Some(l.id.clone()))
                            >{l.name}</button>
                        }
                    }
                />
            </div>

            {move || {
                if let Some(e) = error.get() {
                    view! { <div class="callout callout-err">{e}</div> }.into_any()
                } else if loading.get() && list.get().is_empty() {
                    view! {
                        <div class="poster-grid">
                            {(0..12).map(|_| view! { <div class="poster-card"><div class="skeleton poster-skel"/><div class="skeleton line-skel"/></div> })
                                .collect_view()}
                        </div>
                    }
                    .into_any()
                } else if list.get().is_empty() {
                    view! {
                        <div class="empty">
                            <IconFilm size=34/>
                            <p>"Nothing here."</p>
                            <p class="muted">"Try a different library or search term."</p>
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <div class="poster-grid">
                            <For
                                each=move || list.get()
                                key=|i| i.id.clone()
                                children=move |item| {
                                    let item2 = item.clone();
                                    let open = {
                                        move |_| {
                                            let ctx = sctx.get_value();
                                            let item = item2.clone();
                                            spawn_local(async move {
                                                if item.item_type == "Series" {
                                                    match api::episodes(item.id.clone()).await {
                                                        Ok(eps) => {
                                                            ctx.episodes.set(Some(crate::SeriesEpisodes {
                                                                name: item.name.clone(),
                                                                list: eps,
                                                            }))
                                                        }
                                                        Err(e) => push_toast(&ctx, "err", e),
                                                    }
                                                } else {
                                                    match api::item_detail(item.id.clone()).await {
                                                        Ok(d) => ctx.selected.set(Some(d)),
                                                        Err(e) => push_toast(&ctx, "err", e),
                                                    }
                                                }
                                            });
                                        }
                                    };
                                    view! {
                                        <button class="poster-card" on:click=open>
                                            <div class="poster">
                                                {match item.image_url.clone() {
                                                    Some(u) => view! { <img src=u loading="lazy" alt=item.name.clone() onerror="this.style.display='none'"/> }.into_any(),
                                                    None => view! { <div class="poster-none"><IconFilm size=26/></div> }.into_any(),
                                                }}
                                                <span class="poster-type">
                                                    {if item.item_type == "Series" { "TV" } else { "" }}
                                                </span>
                                            </div>
                                            <div class="poster-title">{item.name.clone()}</div>
                                            <div class="poster-sub">
                                                {format!("{}{}",
                                                    item.year.map(|y| y.to_string()).unwrap_or_default(),
                                                    item.series_name.as_ref()
                                                        .map(|s| format!("{}{}", if item.year.is_some() {" · "} else {""}, s.clone()))
                                                        .unwrap_or_default())}
                                            </div>
                                        </button>
                                    }
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
