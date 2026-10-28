use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;

use crate::format::{format_modified, format_size, now_ms};
use crate::i18n::attr;
use crate::tasks::{FolderSize, Task, use_tasks};

/// A folder's size once a usage scan measured it, in italic; an ellipsis while one runs, and
/// `unknown` otherwise (nothing when empty). `separator` follows a shown size with " · ".
#[component]
pub fn FolderSizeLabel(
    volume: String,
    path: String,
    #[props(default = "—".to_owned())] unknown: String,
    #[props(default)] separator: bool,
) -> Element {
    let list = use_tasks().list();
    let locale = i18n().language();
    // A memo, so that a poll re-renders the label only when its folder's size changed.
    let measured = use_memo(use_reactive!(|volume, path| {
        let list = list.read();
        let finished_at = list
            .newest_usage(&volume, &path)
            .and_then(Task::finished_at_ms);
        (list.folder_size(&volume, &path), finished_at)
    }));
    let (size, finished_at) = measured();
    let tail = if separator { " · " } else { "" };

    match size {
        FolderSize::Known(bytes) => rsx! {
            span {
                class: "italic",
                title: finished_at.map(|ms| attr(t!("folder-size-calculated", time: format_modified(ms, now_ms(), &locale)))),
                {format_size(bytes, &locale)}
            }
            {tail}
        },
        FolderSize::Computing => rsx! {
            span { title: t!("folder-size-computing"),
                span { aria_hidden: "true", "…" }
                span { class: "sr-only", {t!("folder-size-computing")} }
            }
            {tail}
        },
        FolderSize::Unknown if unknown.is_empty() => rsx! {},
        FolderSize::Unknown => rsx! { {unknown} },
    }
}
