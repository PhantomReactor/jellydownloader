use crate::AppCtx;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: &'static str,
    pub text: String,
}

pub fn push_toast(ctx: &AppCtx, kind: &'static str, text: impl Into<String>) {
    let id = crate::next_toast_id();
    ctx.toasts.update(|t| t.push(Toast { id, kind, text: text.into() }));
    let toasts = ctx.toasts;
    spawn_local(async move {
        TimeoutFuture::new(4500).await;
        toasts.update(|t| t.retain(|x| x.id != id));
    });
}

#[component]
pub fn Toasts() -> impl IntoView {
    let ctx = use_context::<AppCtx>().expect("app context");
    view! {
        <div class="toasts">
            <For
                each=move || ctx.toasts.get()
                key=|t| t.id
                children=move |t| {
                    view! {
                        <div class=format!("toast toast-{}", t.kind)>
                            <span>{t.text}</span>
                        </div>
                    }
                }
            />
        </div>
    }
}
