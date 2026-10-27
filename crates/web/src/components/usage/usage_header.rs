use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::tasks::Location;
use shared::api::usage::UsageView;

use crate::components::common::icons::WarningIcon;
use crate::format::{format_count, format_modified, format_size, now_ms};
use crate::routes::{Route, Segments};
use crate::tasks::use_tasks;

const LINK: &str =
    "truncate rounded hover:text-indigo-600 hover:underline dark:hover:text-indigo-400";
const CURRENT: &str = "truncate font-semibold text-gray-900 dark:text-white";
pub const PRIMARY: &str = "inline-flex min-h-10 items-center rounded-md bg-indigo-600 px-3 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:bg-indigo-500 dark:hover:bg-indigo-400";
pub const SECONDARY: &str = "inline-flex min-h-10 items-center rounded-md border border-slate-300 px-3 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";
const WARNING: &str = "flex items-start gap-1.5 text-sm text-amber-700 dark:text-amber-400";

/// The shown folder within the scan, its totals, when it was scanned, and what to do next.
#[component]
pub fn UsageHeader(id: String, view: UsageView) -> Element {
    let tasks = use_tasks();
    let locale = i18n().language();
    let root = Segments::of(&view.root);
    let path = Segments::of(&view.path);
    let crumbs = crumbs(&view.volume, &root, &path);
    let totals = [
        format_size(view.bytes, &locale),
        t!("usage-files", count: view.files, files: format_count(view.files, &locale)),
        t!("usage-folders", count: view.dirs, folders: format_count(view.dirs, &locale)),
    ]
    .join(" · ");
    let scanned = t!(
        "usage-scanned-at",
        time: format_modified(view.scanned_at_ms, now_ms(), &locale)
    );
    let rescan = Location {
        volume: view.volume.clone(),
        path: view.root.clone(),
    };
    let last = crumbs.len() - 1;

    rsx! {
        div { class: "mb-4 flex flex-col gap-2",
            nav { aria_label: t!("usage-breadcrumb"),
                ol { class: "flex min-w-0 flex-wrap items-center gap-x-1.5 text-sm text-slate-500 dark:text-slate-400",
                    for (index, (prefix, name)) in crumbs.into_iter().enumerate() {
                        li { key: "{index}", class: "flex min-w-0 items-center gap-x-1.5",
                            if index > 0 {
                                span { aria_hidden: "true", "›" }
                            }
                            if index == last {
                                h1 { class: CURRENT, aria_current: "page", {name} }
                            } else {
                                Link {
                                    to: Route::Usage { id: id.clone(), volume: view.volume.as_str().into(), path: prefix },
                                    class: LINK,
                                    {name}
                                }
                            }
                        }
                    }
                }
            }
            p { class: "text-sm font-medium text-gray-900 dark:text-slate-100", {totals} }
            p { class: "text-xs text-slate-500 dark:text-slate-400", {scanned} }
            if view.incomplete {
                p { class: WARNING,
                    WarningIcon { class: "mt-0.5 size-4 shrink-0" }
                    {t!("usage-incomplete")}
                }
            }
            if view.collapsed {
                p { class: WARNING,
                    WarningIcon { class: "mt-0.5 size-4 shrink-0" }
                    {t!("usage-collapsed")}
                }
            }
            div { class: "mt-1 flex flex-wrap items-center gap-2",
                button {
                    r#type: "button",
                    class: PRIMARY,
                    onclick: move |_| tasks.analyze(rescan.clone(), true, true),
                    {t!("usage-scan-again")}
                }
                Link {
                    to: Route::Browse { volume: view.volume.as_str().into(), path },
                    class: SECONDARY,
                    {t!("usage-open-in-explorer")}
                }
            }
        }
    }
}

/// The folders from the scanned `root` down to `path`, each with its label: the volume's name
/// for its root.
fn crumbs(volume: &str, root: &Segments, path: &Segments) -> Vec<(Segments, String)> {
    let names = path.names();
    let from = root.names().len().min(names.len());
    (from..=names.len())
        .map(|len| {
            let label = match len {
                0 => volume.to_owned(),
                len => names[len - 1].clone(),
            };
            (path.prefix(len), label)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(volume: &str, root: &str, path: &str) -> Vec<(String, String)> {
        crumbs(volume, &Segments::of(root), &Segments::of(path))
            .into_iter()
            .map(|(prefix, label)| (prefix.joined(), label))
            .collect()
    }

    fn pair(path: &str, label: &str) -> (String, String) {
        (path.to_owned(), label.to_owned())
    }

    #[test]
    fn crumbs_start_at_the_scanned_folder() {
        assert_eq!(labels("media", "Films", "Films"), [pair("Films", "Films")]);
        assert_eq!(
            labels("media", "Films", "Films/2026/x y"),
            [
                pair("Films", "Films"),
                pair("Films/2026", "2026"),
                pair("Films/2026/x y", "x y"),
            ]
        );
        assert_eq!(
            labels("media", "", "Films"),
            [pair("", "media"), pair("Films", "Films")]
        );
        assert_eq!(labels("media", "", ""), [pair("", "media")]);
    }
}
