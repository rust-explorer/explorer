use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::{Entry, EntryKind};

use super::VirtualList;
use super::icons::{
    ArchiveIcon, ArrowDownIcon, ArrowUpIcon, AudioIcon, DocumentIcon, DownloadIcon, FileIcon,
    FolderIcon, ImageIcon, VideoIcon,
};
use crate::api;
use crate::format::{format_modified, format_size, now_ms};
use crate::listing::{FileType, Sort, SortKey, file_type, sorted};
use crate::routes::{Route, Segment, Segments};

const ROW_HEIGHT: f64 = 44.0;
const COLUMNS: &str = "grid grid-cols-[minmax(0,1fr)_2.75rem] items-center gap-x-3 pl-3 md:grid-cols-[minmax(0,1fr)_6rem_9rem_2.75rem]";
const NAME_CLASS: &str = "flex min-w-0 items-center gap-3 self-stretch";
const MUTED: &str = "text-slate-500 dark:text-slate-400";

#[component]
pub fn FileTable(volume: Segment, path: Segments, entries: ReadSignal<Rc<Vec<Entry>>>) -> Element {
    let mut sort = use_context::<Signal<Sort>>();
    let rows = use_memo(move || Rc::new(sorted(&entries.read(), sort())));
    let now = use_memo(move || {
        let _ = entries.read();
        now_ms()
    });
    let total = rows.read().len();
    let on_sort = move |column: SortKey| sort.set(sort().toggled(column));

    rsx! {
        div {
            role: "table",
            aria_label: t!("table-label"),
            aria_rowcount: total + 1,
            class: "flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-slate-200 bg-white text-sm dark:border-slate-800 dark:bg-slate-900",
            div { role: "rowgroup", class: "shrink-0 overflow-hidden border-b border-slate-200 [scrollbar-gutter:stable] dark:border-slate-800",
                div { role: "row", aria_rowindex: 1, class: "{COLUMNS} h-9 text-xs font-medium {MUTED}",
                    HeaderCell { column: SortKey::Name, label: t!("table-name"), sort: sort(), on_sort }
                    HeaderCell { column: SortKey::Size, label: t!("table-size"), sort: sort(), on_sort }
                    HeaderCell { column: SortKey::Modified, label: t!("table-modified"), sort: sort(), on_sort }
                    div { role: "columnheader", span { class: "sr-only", {t!("table-download")} } }
                }
            }
            if total == 0 {
                p { class: "py-10 text-center {MUTED}", {t!("browse-empty")} }
            } else {
                VirtualList {
                    role: "rowgroup",
                    class: "min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]",
                    total,
                    row_height: ROW_HEIGHT,
                    row: move |index: usize| {
                        let entry = rows.read()[index].clone();
                        rsx! {
                            FileRow {
                                volume: volume.clone(),
                                path: path.clone(),
                                entry,
                                index,
                                now: now(),
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn HeaderCell(
    column: SortKey,
    label: String,
    sort: Sort,
    on_sort: EventHandler<SortKey>,
) -> Element {
    let active = sort.key == column;
    let aria_sort = match (active, sort.ascending) {
        (false, _) => None,
        (true, true) => Some("ascending"),
        (true, false) => Some("descending"),
    };
    let visibility = if column == SortKey::Name {
        "flex"
    } else {
        "hidden md:flex"
    };

    rsx! {
        div { role: "columnheader", aria_sort, class: "{visibility} min-w-0",
            button {
                r#type: "button",
                class: "-mx-1 inline-flex items-center gap-1 rounded px-1 py-0.5 hover:text-slate-900 focus-visible:outline-2 focus-visible:outline-indigo-600 dark:hover:text-white",
                onclick: move |_| on_sort.call(column),
                {label}
                if active {
                    if sort.ascending {
                        ArrowUpIcon {}
                    } else {
                        ArrowDownIcon {}
                    }
                }
            }
        }
    }
}

#[component]
fn FileRow(volume: Segment, path: Segments, entry: Entry, index: usize, now: i64) -> Element {
    let locale = i18n().language();
    let size = entry
        .size
        .filter(|_| entry.kind == EntryKind::File)
        .map(|size| format_size(size, &locale));
    let modified = entry
        .modified_ms
        .map(|ms| format_modified(ms, now, &locale));
    let details = [size.clone(), modified.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let child = path.child(&entry.name);
    let disabled = entry.kind == EntryKind::Other;
    let row_class = if disabled {
        "text-slate-400 dark:text-slate-600"
    } else {
        "hover:bg-slate-50 dark:hover:bg-slate-800/60"
    };

    let label = rsx! {
        EntryIcon { kind: entry.kind, name: entry.name.clone() }
        span { class: "flex min-w-0 flex-col",
            span { class: "truncate", {entry.name.clone()} }
            if !details.is_empty() {
                span { class: "truncate text-xs md:hidden {MUTED}", {details} }
            }
        }
    };

    rsx! {
        div {
            role: "row",
            aria_rowindex: index + 2,
            aria_disabled: if disabled { "true" },
            title: if disabled { t!("entry-unavailable") },
            class: "group {COLUMNS} h-11 border-b border-slate-100 {row_class} dark:border-slate-800",
            div { role: "cell", class: "flex min-w-0 self-stretch",
                match entry.kind {
                    EntryKind::Dir => rsx! {
                        Link {
                            to: Route::Browse { volume: volume.clone(), path: child.clone() },
                            class: "{NAME_CLASS} focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indigo-600",
                            {label}
                        }
                    },
                    EntryKind::File => rsx! {
                        a {
                            href: api::download_url(volume.as_str(), &child.joined()),
                            download: "",
                            class: "{NAME_CLASS} focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-indigo-600",
                            {label}
                        }
                    },
                    EntryKind::Other => rsx! {
                        span { class: NAME_CLASS,
                            {label}
                            span { class: "sr-only", {t!("entry-unavailable")} }
                        }
                    },
                }
            }
            div { role: "cell", class: "hidden truncate md:block {MUTED}", {size.unwrap_or_else(|| "—".to_owned())} }
            div { role: "cell", class: "hidden truncate md:block {MUTED}", {modified.unwrap_or_else(|| "—".to_owned())} }
            div { role: "cell", class: "flex justify-center",
                if let Some(href) = download_href(volume.as_str(), &child, entry.kind) {
                    a {
                        href,
                        download: "",
                        aria_label: if entry.kind == EntryKind::Dir {
                            t!("download-folder", name: entry.name.clone())
                        } else {
                            t!("download-file", name: entry.name.clone())
                        },
                        class: "inline-flex size-8 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-900 focus-visible:outline-2 focus-visible:outline-indigo-600 pointer-fine:opacity-0 pointer-fine:group-hover:opacity-100 pointer-fine:group-focus-within:opacity-100 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white",
                        DownloadIcon {}
                    }
                }
            }
        }
    }
}

#[component]
fn EntryIcon(kind: EntryKind, name: String) -> Element {
    const CLASS: &str = "size-5 shrink-0";
    const FILE: &str = "size-5 shrink-0 text-slate-400 dark:text-slate-500";
    match kind {
        EntryKind::Dir => rsx! {
            FolderIcon { class: "{CLASS} text-indigo-500 dark:text-indigo-400" }
        },
        EntryKind::Other => rsx! {
            FileIcon { class: CLASS }
        },
        EntryKind::File => match file_type(&name) {
            FileType::Video => rsx! { VideoIcon { class: FILE } },
            FileType::Image => rsx! { ImageIcon { class: FILE } },
            FileType::Audio => rsx! { AudioIcon { class: FILE } },
            FileType::Archive => rsx! { ArchiveIcon { class: FILE } },
            FileType::Document => rsx! { DocumentIcon { class: FILE } },
            FileType::Other => rsx! { FileIcon { class: FILE } },
        },
    }
}

/// A file downloads as is, a folder as a zip; other entries can't be downloaded.
fn download_href(volume: &str, path: &Segments, kind: EntryKind) -> Option<String> {
    match kind {
        EntryKind::Dir => Some(api::zip_url(volume, &path.joined())),
        EntryKind::File => Some(api::download_url(volume, &path.joined())),
        EntryKind::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_link_to_children_with_names_round_tripping() {
        let path = Segments::from(vec!["Films".to_owned()]);
        let child = path.child("é #?%😀.mkv");
        assert_eq!(
            download_href("media", &child, EntryKind::File).as_deref(),
            Some("/files/download?volume=media&path=Films%2F%C3%A9%20%23%3F%25%F0%9F%98%80.mkv")
        );
        assert_eq!(
            download_href("media", &child, EntryKind::Dir).as_deref(),
            Some("/files/zip?volume=media&path=Films%2F%C3%A9%20%23%3F%25%F0%9F%98%80.mkv")
        );
        assert_eq!(download_href("media", &child, EntryKind::Other), None);

        let route = Route::Browse {
            volume: "media".into(),
            path: path.child("100% a#b"),
        };
        let url = route.to_string();
        assert_eq!(url, "/browse/media/Films/100%25%20a%23b");
        assert_eq!(url.parse::<Route>().ok(), Some(route));
    }
}
