use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::tasks::Location;

use crate::components::common::icons::ChartIcon;
use crate::tasks::use_tasks;

const BUTTON: &str = "size-10 shrink-0 items-center justify-center rounded-md text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:text-slate-200 dark:hover:bg-slate-800";

/// Opens the space usage of `volume`/`path`, scanning it first when needed. `class` sets where
/// it shows, such as `inline-flex` or `hidden md:inline-flex`.
#[component]
pub fn AnalyzeButton(volume: String, path: String, class: &'static str) -> Element {
    let tasks = use_tasks();
    rsx! {
        button {
            r#type: "button",
            class: "{BUTTON} {class}",
            aria_label: t!("analyze-usage"),
            title: t!("analyze-usage"),
            onclick: move |_| {
                tasks.analyze(
                    Location { volume: volume.clone(), path: path.clone() },
                    true,
                    false,
                );
            },
            ChartIcon {}
        }
    }
}
