use crate::{api, toast::push_toast, AppCtx};
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn LoginPage() -> impl IntoView {
    let ctx = use_context::<AppCtx>().expect("app context");
    let (username, set_username) = signal(String::new());
    let (password, set_password) = signal(String::new());
    let (error, set_error) = signal(None::<String>);
    let busy = RwSignal::new(false);

    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if busy.get() {
            return;
        }
        busy.set(true);
        set_error.try_set(None);
        let ctx = ctx.clone();
        let user = username.get().trim().to_string();
        let pass = password.get();
        spawn_local(async move {
            match api::login(user, pass).await {
                Ok(u) => {
                    push_toast(&ctx, "ok", format!("Signed in as {}", u.name));
                    ctx.user.set(Some(u));
                }
                Err(msg) => {
                    if error.try_get().is_some() {
                        set_error.set(Some(msg));
                    }
                }
            }
            if busy.try_get().is_some() {
                busy.set(false);
            }
        });
    };

    view! {
        <div class="login-wrap">
            <form class="login-card" on:submit=submit>
                <div class="login-brand">
                    <span class="brand-mark"></span>
                    <h1>"jellydown"</h1>
                    <p>"Sign in with your Jellyfin credentials"</p>
                </div>
                <label class="field">
                    <span>"Username"</span>
                    <input
                        type="text"
                        placeholder="you"
                        prop:value=move || username.get()
                        on:input=move |e| set_username.set(event_target_value(&e))
                        autocomplete="username"
                        autofocus
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
                <p class="login-hint">"The Jellyfin server is configured on the app host."</p>
            </form>
        </div>
    }
}
