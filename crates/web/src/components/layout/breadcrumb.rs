use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::routes::{Route, Segment, Segments};

const LINK_CLASS: &str =
    "truncate rounded hover:text-indigo-600 hover:underline dark:hover:text-indigo-400";
const CURRENT_CLASS: &str = "truncate font-semibold text-gray-900 dark:text-white";

#[component]
pub fn Breadcrumb(volume: Segment, path: Segments) -> Element {
    let names = path.names();

    rsx! {
        nav { aria_label: t!("breadcrumb"),
            ol { class: "flex min-w-0 flex-wrap items-center gap-x-1.5 text-sm text-slate-500 dark:text-slate-400",
                li { class: "flex min-w-0 items-center",
                    if names.is_empty() {
                        span { class: CURRENT_CLASS, aria_current: "page", {volume.as_str()} }
                    } else {
                        Link {
                            to: Route::Browse { volume: volume.clone(), path: Segments::default() },
                            class: LINK_CLASS,
                            {volume.as_str()}
                        }
                    }
                }
                for (index, name) in names.iter().enumerate() {
                    li { key: "{index}", class: "flex min-w-0 items-center gap-x-1.5",
                        span { aria_hidden: "true", "›" }
                        if index + 1 == names.len() {
                            span { class: CURRENT_CLASS, aria_current: "page", {name.clone()} }
                        } else {
                            Link {
                                to: Route::Browse { volume: volume.clone(), path: path.prefix(index + 1) },
                                class: LINK_CLASS,
                                {name.clone()}
                            }
                        }
                    }
                }
            }
        }
    }
}
