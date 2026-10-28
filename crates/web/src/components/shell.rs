use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::Me;

use super::user_menu::UserMenu;

/// Page frame shared by every route; the user menu appears once the session is known.
#[component]
pub fn Shell(user: Option<Me>, children: Element) -> Element {
    rsx! {
        div { class: "flex min-h-screen flex-col bg-slate-50 text-gray-800 dark:bg-slate-950 dark:text-slate-200",
            header { class: "flex h-12 items-center gap-2 border-b border-slate-200 bg-white px-4 dark:border-slate-800 dark:bg-slate-900",
                span { class: "inline-block size-3 rounded-sm bg-indigo-600 dark:bg-indigo-400" }
                span { class: "font-bold text-gray-900 dark:text-white", {t!("app-name")} }
                if let Some(user) = user {
                    div { class: "ml-auto", UserMenu { user } }
                }
            }
            main { class: "flex flex-1 items-center justify-center p-6", {children} }
        }
    }
}
