use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::common::ICON_BUTTON;
use crate::components::common::icons::{CloseIcon, MinusIcon, PlusIcon};
use crate::components::tasks::TaskRows;
use crate::components::uploads::{UploadRows, active_uploads};
use crate::tasks::use_tasks;
use crate::uploads::use_uploads;

const LIST_ID: &str = "activity-list";

/// Floating panel with the tab's background tasks, then its uploads.
#[component]
pub fn ActivityPanel() -> Element {
    let uploads = use_uploads();
    let tasks = use_tasks();
    let mut collapsed = use_signal(|| false);
    let queue = uploads.queue();
    let list = tasks.list();
    let (upload_count, skipped, active_upload_count) = {
        let queue = queue.read();
        (
            queue.items().len(),
            queue.skipped(),
            active_uploads(queue.items()),
        )
    };
    let (task_count, active_task_count) = {
        let list = list.read();
        (list.shown().count(), list.active_count())
    };
    if upload_count == 0 && skipped == 0 && task_count == 0 {
        return rsx! {};
    }
    let active = active_upload_count + active_task_count;
    let collapse_label = if collapsed() {
        t!("activity-expand")
    } else {
        t!("activity-collapse")
    };

    rsx! {
        section {
            class: "fixed inset-x-4 bottom-20 z-30 rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm shadow-lg md:inset-x-auto md:bottom-4 md:right-4 md:w-80 dark:border-slate-700 dark:bg-slate-900",
            aria_label: t!("activity-title", count: active),
            div { class: "flex items-center gap-1",
                h2 { class: "flex-1 font-semibold text-gray-900 dark:text-white",
                    {t!("activity-title", count: active)}
                }
                button {
                    r#type: "button",
                    class: ICON_BUTTON,
                    aria_label: "{collapse_label}",
                    title: "{collapse_label}",
                    aria_expanded: !collapsed(),
                    aria_controls: LIST_ID,
                    onclick: move |_| collapsed.toggle(),
                    if collapsed() {
                        PlusIcon { class: "size-4" }
                    } else {
                        MinusIcon {}
                    }
                }
                if active == 0 {
                    button {
                        r#type: "button",
                        class: ICON_BUTTON,
                        aria_label: t!("activity-close"),
                        title: t!("activity-close"),
                        onclick: move |_| {
                            uploads.clear_finished();
                            tasks.clear_finished();
                        },
                        CloseIcon {}
                    }
                }
            }
            if skipped > 0 {
                p { class: "mt-1 text-xs text-amber-700 dark:text-amber-400",
                    {t!("uploads-unreadable", count: skipped)}
                }
            }
            if !collapsed() {
                ul { id: LIST_ID, class: "mt-1 max-h-80 overflow-y-auto",
                    TaskRows {}
                    UploadRows {}
                }
            }
        }
    }
}
