use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::Me;

use crate::api;
use crate::components::Shell;

#[component]
pub fn Home(segments: Vec<String>) -> Element {
    let _ = segments;
    let mut me = use_resource(|| api::get_json::<Me>("/api/me"));

    let result = me.read().clone();
    match result {
        Some(Ok(user)) => rsx! {
            Shell { user,
                p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("app-tagline")} }
            }
        },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            Shell {
                div { class: "flex flex-col items-center gap-3",
                    p { role: "alert", class: "text-sm text-slate-600 dark:text-slate-400", {t!("load-failed")} }
                    button {
                        r#type: "button",
                        class: "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500",
                        onclick: move |_| me.restart(),
                        {t!("try-again")}
                    }
                }
            }
        },
        // Still loading, or already redirecting to the sign-in page.
        _ => rsx! {
            Shell {
                p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} }
            }
        },
    }
}
