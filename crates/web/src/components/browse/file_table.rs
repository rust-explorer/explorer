use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};

use super::row_menu::{Anchor, MenuAt, MenuRequest, ROW_MENU_ID, RowMenu, menu_actions};
use super::virtual_list::VirtualList;
use crate::actions::RowAction;
use crate::api;
use crate::components::RenameDialog;
use crate::components::common::FocusTarget;
use crate::components::common::icons::{
    ArchiveIcon, ArrowDownIcon, ArrowUpIcon, AudioIcon, DocumentIcon, DownloadIcon, FileIcon,
    FolderIcon, ImageIcon, MoreIcon, VideoIcon,
};
use crate::format::{format_modified, format_size, now_ms};
use crate::listing::{FileType, Sort, SortKey, file_type, sorted};
use crate::routes::{Route, Segment, Segments};

const ROW_HEIGHT: f64 = 44.0;
const COLUMNS: &str = "grid grid-cols-[minmax(0,1fr)_4.75rem] items-center gap-x-3 pl-3 md:grid-cols-[minmax(0,1fr)_6rem_9rem_4.75rem]";
const ROW_BUTTON: &str = "inline-flex size-8 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-900 focus-visible:outline-2 focus-visible:outline-indigo-600 pointer-fine:opacity-0 pointer-fine:group-hover:opacity-100 pointer-fine:group-focus-within:opacity-100 pointer-fine:aria-expanded:opacity-100 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white";
const NAME_CLASS: &str = "flex min-w-0 items-center gap-3 self-stretch";
const MUTED: &str = "text-slate-500 dark:text-slate-400";

/// A rename in progress and the ⋯ button that gets focus back.
#[derive(Clone, PartialEq)]
struct Renaming {
    entry: Entry,
    trigger: Option<FocusTarget>,
}

#[component]
pub fn FileTable(
    volume: Segment,
    path: Segments,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    read_only: bool,
) -> Element {
    let mut sort = use_context::<Signal<Sort>>();
    let mut menu = use_signal(|| None::<MenuRequest>);
    let mut renaming = use_signal(|| None::<Renaming>);
    let rows = use_memo(move || Rc::new(sorted(&entries.read(), sort())));
    let now = use_memo(move || {
        let _ = entries.read();
        now_ms()
    });
    let total = rows.read().len();
    let on_sort = move |column: SortKey| sort.set(sort().toggled(column));
    let menu_name = menu.read().as_ref().map(|open| open.entry.name.clone());
    let (dialog_volume, dialog_parent) = (volume.as_str().to_owned(), path.joined());

    let refocus = move |trigger: Option<FocusTarget>| {
        if let Some(FocusTarget(button)) = trigger {
            spawn(async move {
                let _ = button.set_focus(true).await;
            });
        }
    };
    let on_action = {
        let (volume, path) = (volume.clone(), path.clone());
        move |action: RowAction| {
            let Some(MenuRequest { entry, trigger, .. }) = menu.take() else {
                return;
            };
            match action {
                RowAction::Rename => renaming.set(Some(Renaming { entry, trigger })),
                RowAction::Download => {
                    if let Some(href) =
                        download_href(volume.as_str(), &path.child(&entry.name), entry.kind)
                    {
                        api::navigate(&href);
                    }
                    refocus(trigger);
                }
                RowAction::MoveTo | RowAction::CopyTo | RowAction::Delete => refocus(trigger),
            }
        }
    };

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
                    div { role: "columnheader", span { class: "sr-only", {t!("table-actions")} } }
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
                        let menu_open = menu_name.as_deref() == Some(entry.name.as_str());
                        let has_menu = !menu_actions(&entry, read_only).is_empty();
                        rsx! {
                            FileRow {
                                volume: volume.clone(),
                                path: path.clone(),
                                entry,
                                index,
                                now: now(),
                                has_menu,
                                menu_open,
                                on_menu: move |request| menu.set(Some(request)),
                            }
                        }
                    },
                }
            }
        }
        if let Some(request) = menu() {
            RowMenu {
                request,
                read_only,
                on_close: move |()| {
                    if let Some(open) = menu.take() {
                        refocus(open.trigger);
                    }
                },
                on_action,
            }
        }
        if let Some(Renaming { entry, trigger }) = renaming() {
            RenameDialog {
                volume: dialog_volume,
                parent: dialog_parent,
                entry,
                entries,
                return_focus: trigger,
                on_close: move |()| renaming.set(None),
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
fn FileRow(
    volume: Segment,
    path: Segments,
    entry: Entry,
    index: usize,
    now: i64,
    has_menu: bool,
    menu_open: bool,
    on_menu: EventHandler<MenuRequest>,
) -> Element {
    let mut more = use_signal(|| None::<Rc<MountedData>>);
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

    let open_at = {
        let entry = entry.clone();
        move |at: MenuAt| {
            on_menu.call(MenuRequest {
                entry: entry.clone(),
                at,
                trigger: more().map(FocusTarget),
            });
        }
    };
    let on_context_menu = {
        let open_at = open_at.clone();
        move |event: MouseEvent| {
            if has_menu {
                event.prevent_default();
                let point = event.client_coordinates();
                open_at(MenuAt::Pointer {
                    x: point.x,
                    y: point.y,
                });
            }
        }
    };
    let on_more = move |event: MouseEvent| {
        let point = event.client_coordinates();
        let fallback = MenuAt::Pointer {
            x: point.x,
            y: point.y,
        };
        let open_at = open_at.clone();
        let button = more();
        spawn(async move {
            let at = match button {
                Some(button) => button.get_client_rect().await.map_or(fallback, |rect| {
                    MenuAt::Below(Anchor {
                        left: rect.min_x(),
                        top: rect.min_y(),
                        right: rect.max_x(),
                        bottom: rect.max_y(),
                    })
                }),
                None => fallback,
            };
            open_at(at);
        });
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
            oncontextmenu: on_context_menu,
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
            div { role: "cell", class: "flex justify-end gap-1 pr-1",
                if let Some(href) = download_href(volume.as_str(), &child, entry.kind) {
                    a {
                        href,
                        download: "",
                        aria_label: if entry.kind == EntryKind::Dir {
                            t!("download-folder", name: entry.name.clone())
                        } else {
                            t!("download-file", name: entry.name.clone())
                        },
                        class: ROW_BUTTON,
                        DownloadIcon {}
                    }
                }
                if has_menu {
                    button {
                        r#type: "button",
                        class: ROW_BUTTON,
                        aria_label: t!("row-menu-open", name: entry.name.clone()),
                        aria_haspopup: "menu",
                        aria_expanded: menu_open,
                        aria_controls: if menu_open { ROW_MENU_ID },
                        onmounted: move |event| more.set(Some(event.data())),
                        onclick: on_more,
                        MoreIcon {}
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
