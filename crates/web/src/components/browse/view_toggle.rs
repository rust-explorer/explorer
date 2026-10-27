use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::common::icons::{GridIcon, ListIcon};
use crate::view_mode::{ViewMode, set_view_mode};

const BUTTON: &str = "inline-flex size-10 items-center justify-center rounded-md text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 aria-pressed:bg-slate-200 aria-pressed:text-slate-900 dark:text-slate-200 dark:hover:bg-slate-800 dark:aria-pressed:bg-slate-700 dark:aria-pressed:text-white";

/// Switches the browse page between the list and the grid.
#[component]
pub fn ViewToggle() -> Element {
    let mode = use_context::<Signal<ViewMode>>();
    let current = mode();

    rsx! {
        div { role: "group", aria_label: t!("view-mode"), class: "flex items-center",
            for (value, label) in [(ViewMode::List, t!("view-list")), (ViewMode::Grid, t!("view-grid"))] {
                button {
                    key: "{value.as_str()}",
                    r#type: "button",
                    class: BUTTON,
                    aria_label: label.clone(),
                    title: label,
                    aria_pressed: current == value,
                    onclick: move |_| set_view_mode(mode, value),
                    if value == ViewMode::List {
                        ListIcon {}
                    } else {
                        GridIcon {}
                    }
                }
            }
        }
    }
}
