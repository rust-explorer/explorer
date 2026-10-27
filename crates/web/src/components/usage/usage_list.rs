use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::EntryKind;
use shared::api::usage::{UsageItem, UsageKind, UsageView};

use crate::components::browse::EntryIcon;
use crate::components::common::icons::WarningIcon;
use crate::format::{format_count, format_size};
use crate::routes::{Route, Segments};

const MUTED: &str = "text-slate-500 dark:text-slate-400";

/// The folder's items, biggest first, each with its share of the folder's total. A folder's
/// whole row opens it.
#[component]
pub fn UsageList(id: String, view: UsageView) -> Element {
    if view.items.is_empty() {
        return rsx! {
            p { class: "text-sm {MUTED}", {t!("usage-empty")} }
        };
    }
    let path = Segments::of(&view.path);

    rsx! {
        ul {
            aria_label: t!("usage-items"),
            class: "overflow-hidden rounded-lg border border-slate-200 bg-white text-sm dark:border-slate-800 dark:bg-slate-900",
            for (index, item) in view.items.iter().cloned().enumerate() {
                li {
                    key: "{index}-{item.name}",
                    class: "relative flex min-h-12 items-center gap-3 border-b border-slate-100 px-3 py-2 last:border-b-0 dark:border-slate-800",
                    ItemRow {
                        id: id.clone(),
                        volume: view.volume.clone(),
                        path: path.clone(),
                        item,
                        total: view.bytes,
                    }
                }
            }
        }
    }
}

#[component]
fn ItemRow(id: String, volume: String, path: Segments, item: UsageItem, total: u64) -> Element {
    let locale = i18n().language();
    let (kind, name) = match item.kind {
        UsageKind::Dir => (EntryKind::Dir, item.name.clone()),
        UsageKind::File => (EntryKind::File, item.name.clone()),
        UsageKind::OtherFiles => (
            EntryKind::File,
            t!("usage-other-files", count: item.files, files: format_count(item.files, &locale)),
        ),
    };
    let percent = match share(item.bytes, total) {
        Share::None => String::new(),
        Share::Small => t!("usage-percent-small"),
        Share::Whole(percent) => t!("usage-percent", percent: percent),
    };

    rsx! {
        EntryIcon { kind, name: name.clone() }
        div { class: "flex min-w-0 flex-1 flex-col gap-1",
            div { class: "flex min-w-0 items-center gap-1.5",
                if item.kind == UsageKind::Dir {
                    Link {
                        to: Route::Usage { id, volume: volume.into(), path: path.child(&item.name) },
                        class: "truncate rounded text-gray-900 after:absolute after:inset-0 hover:text-indigo-600 hover:underline focus-visible:outline-2 focus-visible:outline-indigo-600 dark:text-slate-100 dark:hover:text-indigo-400",
                        {name}
                    }
                } else {
                    span { class: "truncate text-gray-900 dark:text-slate-100", {name} }
                }
                if item.incomplete {
                    span {
                        class: "shrink-0 text-amber-600 dark:text-amber-400",
                        title: t!("usage-incomplete"),
                        WarningIcon { class: "size-3.5" }
                        span { class: "sr-only", {t!("usage-incomplete")} }
                    }
                }
            }
            div {
                class: "h-1.5 w-full rounded-full bg-slate-100 dark:bg-slate-800",
                aria_hidden: "true",
                div {
                    class: "h-1.5 rounded-full bg-indigo-500 dark:bg-indigo-400",
                    width: bar_width(item.bytes, total),
                }
            }
        }
        span { class: "w-20 shrink-0 whitespace-nowrap text-right tabular-nums {MUTED}", {format_size(item.bytes, &locale)} }
        span { class: "w-12 shrink-0 whitespace-nowrap text-right tabular-nums {MUTED}", {percent} }
    }
}

/// An item's part of its folder, rounded to a whole percent.
#[derive(Debug, PartialEq, Eq)]
enum Share {
    /// Empty, or an empty folder.
    None,
    /// Not empty, but under half a percent.
    Small,
    Whole(u64),
}

fn share(bytes: u64, total: u64) -> Share {
    if bytes == 0 || total == 0 {
        return Share::None;
    }
    let percent = (u128::from(bytes) * 200 + u128::from(total)) / (u128::from(total) * 2);
    match u64::try_from(percent.min(100)) {
        Ok(0) => Share::Small,
        Ok(percent) => Share::Whole(percent),
        Err(_) => Share::Whole(100),
    }
}

/// The bar's CSS width: the item's part of the folder's total, at least 2px when not empty.
fn bar_width(bytes: u64, total: u64) -> String {
    if bytes == 0 || total == 0 {
        return "0".to_owned();
    }
    let hundredths = (u128::from(bytes) * 10_000 / u128::from(total)).min(10_000);
    format!("max(2px, {}.{:02}%)", hundredths / 100, hundredths % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_rounds_to_whole_percents() {
        assert_eq!(share(0, 100), Share::None);
        assert_eq!(share(5, 0), Share::None);
        assert_eq!(share(1, 1000), Share::Small);
        assert_eq!(share(5, 1000), Share::Whole(1));
        assert_eq!(share(125, 1000), Share::Whole(13));
        assert_eq!(share(1000, 1000), Share::Whole(100));
        assert_eq!(share(u64::MAX, u64::MAX), Share::Whole(100));
    }

    #[test]
    fn bars_are_a_share_of_the_folder_total() {
        assert_eq!(bar_width(0, 100), "0");
        assert_eq!(bar_width(1, 3), "max(2px, 33.33%)");
        assert_eq!(bar_width(1, 1_000_000), "max(2px, 0.00%)");
        assert_eq!(bar_width(50, 50), "max(2px, 100.00%)");
        // A collapsed folder lists less than its total: the bars do not fill up.
        assert_eq!(bar_width(30, 100), "max(2px, 30.00%)");
    }
}
