use leptos::prelude::*;

macro_rules! icon {
    ($name:ident, $body:expr) => {
        #[component]
        pub fn $name(#[prop(default = 18)] size: u32) -> impl IntoView {
            view! {
                <svg
                    class="icon"
                    width=size
                    height=size
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                >
                    {$body}
                </svg>
            }
        }
    };
}

icon!(IconSearch, view! { <circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/> });
icon!(IconDownload, view! { <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/> });
icon!(IconPause, view! { <rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/> });
icon!(IconPlay, view! { <path d="m6 4 14 8-14 8Z"/> });
icon!(IconStop, view! { <rect x="5" y="5" width="14" height="14" rx="2"/> });
icon!(IconRestart, view! { <path d="M3 12a9 9 0 1 0 3-6.7"/><path d="M3 3v6h6"/> });
icon!(IconTrash, view! { <path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/> });
icon!(IconFilm, view! { <rect x="3" y="3" width="18" height="18" rx="3"/><path d="M7 3v18M17 3v18M3 8h18M3 16h18"/> });
icon!(IconTv, view! { <rect x="3" y="5" width="18" height="13" rx="2"/><path d="m8 21 4-3 4 3"/> });
icon!(IconX, view! { <path d="M18 6 6 18M6 6l12 12"/> });
icon!(IconMenu, view! { <path d="M4 6h16M4 12h16M4 18h16"/> });
icon!(IconLogout, view! { <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><path d="m16 17 5-5-5-5"/><path d="M21 12H9"/> });
icon!(IconCheck, view! { <path d="M20 6 9 17l-5-5"/> });
