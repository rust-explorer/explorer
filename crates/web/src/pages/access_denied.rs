use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::{LanguagePicker, Notice, Shell, SignOutButton};

#[component]
pub fn AccessDenied() -> Element {
    rsx! {
        Shell {
            div { class: "flex w-full max-w-md flex-col gap-4",
                Notice {
                    title: t!("access-denied-title"),
                    message: t!("access-denied-message"),
                    SignOutButton { class: "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 disabled:opacity-60" }
                }
                div { class: "max-w-48",
                    LanguagePicker { id: "access-denied-language" }
                }
            }
        }
    }
}
