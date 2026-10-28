use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::{Notice, Shell};
use crate::routes::Route;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let _ = segments;
    rsx! {
        Shell {
            Notice { title: t!("not-found-title"), message: t!("not-found-message"),
                Link {
                    to: Route::Home {},
                    class: "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500",
                    {t!("error-go-home")}
                }
            }
        }
    }
}
