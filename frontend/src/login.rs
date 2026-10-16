use crate::{api, toast::push_toast, AppCtx};
use leptos::prelude::*;
use leptos::task::spawn_local;

const SERVER_KEY: &str = "jd.server";

fn stored_server() -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SERVER_KEY).ok().flatten())
        .unwrap_or_else(|| "http://192.168.1.100:8096".to_string())
}

#[component]
pub fn LoginPage() -> impl IntoView {
    let ctx = use_context::<AppCtx>().expect("app context");
    let (server, set_server) = signal(stored_server());
    let (username, set_username) = signal(String::new());
    let (password, set_password) = signal(String::new());
    let (error, set_error) = signal(None::<String>);
    let (busy, set_busy) = signal(false);

    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if busy.get() {
            return;
        }
        set_busy.try_set(true);
        set_error.try_set(None);
        let ctx = ctx.clone();
        let server_url = server.get().trim().to_string();
        let user = username.get().trim().to_string();
        let pass = password.get();
        spawn_local(async move {
            match api::login(server_url.clone(), user, pass).await {
                Ok(u) => {
                    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
                        let _ = storage.set_item(SERVER_KEY, &server_url);
                    }
                    push_toast(&ctx, "ok", format!("Signed in as {}", u.name));
                    ctx.user.set(Some(u));
                }
                Err(msg) => {
                    // The page may have navigated away while awaiting.
                    if error.try_get().is_some() {
                        set_error.set(Some(msg));
                    }
                }
            }
            if busy.try_get().is_some() {
                set_busy.set(false);
            }
        });
    };

    view! {
        <div class="login-wrap">
            <form class="login-card" on:submit=submit>
                <div class="login-brand">
                    <span class="brand-mark"></span>
                    <h1>"jellydown"</h1>
                    <p>"Sign in to your Jellyfin server"</p>
                </div>
                <label class="field">
                    <span>"Server URL"</span>
                    <input
                        type="text"
                        placeholder="http://192.168.1.100:8096"
                        prop:value=move || server.get()
                        on:input=move |e| set_server.set(event_target_value(&e))
                        autocomplete="url"
                        spellcheck="false"
                    />
                </label>
                <label class="field">
                    <span>"Username"</span>
                    <input
                        type="text"
                        placeholder="you"
                        prop:value=move || username.get()
                        on:input=move |e| set_username.set(event_target_value(&e))
                        autocomplete="username"
                    />
                </label>
                <label class="field">
                    <span>"Password"</span>
                    <input
                        type="password"
                        placeholder="••••••••"
                        prop:value=move || password.get()
                        on:input=move |e| set_password.set(event_target_value(&e))
                        autocomplete="current-password"
                    />
                </label>
                <Show when=move || error.get().is_some()>
                    <div class="login-error">{move || error.get().unwrap_or_default()}</div>
                </Show>
                <button class="btn btn-primary btn-block" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { "Connecting…" } else { "Connect" }}
                </button>
                <p class="login-hint">"Local or public URL both work. It's stored in this browser only."</p>
            </form>
        </div>
    }
}
