use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::tasks::{Location, TaskSpec};

use super::form::BUTTON;
use crate::components::common::{FocusTarget, Modal};
use crate::tasks::use_tasks;

const TITLE_ID: &str = "delete-title";
const DANGER: &str = "rounded-md bg-red-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-red-600";

/// Asks before deleting `names` from `parent`, then starts one background task per entry.
#[component]
pub fn DeleteDialog(
    volume: String,
    parent: String,
    names: Vec<String>,
    on_close: EventHandler<()>,
    on_confirmed: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
    let tasks = use_tasks();
    let count = names.len();
    let first = names.first().cloned().unwrap_or_default();
    let confirm = move |_| {
        tasks.start(delete_specs(&volume, &parent, &names));
        on_confirmed.call(());
    };

    rsx! {
        Modal { labelled_by: TITLE_ID, on_close: move |()| on_close.call(()), return_focus,
            h2 {
                id: TITLE_ID,
                class: "break-words text-sm font-medium text-gray-900 dark:text-white",
                {t!("delete-title", count: count, name: first)}
            }
            p { class: "mt-2 text-sm text-slate-600 dark:text-slate-400", {t!("delete-warning")} }
            div { class: "mt-5 flex justify-end gap-3",
                button {
                    r#type: "button",
                    class: BUTTON,
                    autofocus: true,
                    onclick: move |_| on_close.call(()),
                    {t!("cancel")}
                }
                button { r#type: "button", class: DANGER, onclick: confirm, {t!("delete-confirm")} }
            }
        }
    }
}

fn delete_specs(volume: &str, parent: &str, names: &[String]) -> Vec<TaskSpec> {
    names
        .iter()
        .map(|name| TaskSpec::Delete {
            source: Location {
                volume: volume.to_owned(),
                path: if parent.is_empty() {
                    name.clone()
                } else {
                    format!("{parent}/{name}")
                },
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(specs: &[TaskSpec]) -> Vec<&str> {
        specs
            .iter()
            .map(|spec| spec.source().path.as_str())
            .collect()
    }

    #[test]
    fn one_task_per_entry() {
        let names = ["a b".to_owned(), "Sub".to_owned()];
        let specs = delete_specs("media", "Films/2026", &names);
        assert_eq!(paths(&specs), ["Films/2026/a b", "Films/2026/Sub"]);
        assert!(specs.iter().all(|spec| spec.source().volume == "media"));
        assert_eq!(paths(&delete_specs("media", "", &names)), ["a b", "Sub"]);
    }
}
