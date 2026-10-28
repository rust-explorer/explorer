use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};

use super::row_menu::{Anchor, MenuAt, MenuRequest, ROW_MENU_ID};
use super::select_cell::SelectCell;
use crate::api;
use crate::components::common::FocusTarget;
use crate::components::common::icons::{
    ArchiveIcon, AudioIcon, DocumentIcon, DownloadIcon, FileIcon, FolderIcon, ImageIcon, MoreIcon,
    VideoIcon,
};
use crate::format::{format_modified, format_size};
use crate::listing::{FileType, file_type};
use crate::routes::{Route, Segment, Segments};

const ROW_BUTTON: &str = "inline-flex size-8 items-center justify-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-900 focus-visible:outline-2 focus-visible:outline-indigo-600 pointer-fine:opacity-0 pointer-fine:group-hover:opacity-100 pointer-fine:group-focus-within:opacity-100 pointer-fine:aria-expanded:opacity-100 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white";
const NAME_CLASS: &str = "flex min-w-0 items-center gap-3 self-stretch";
pub const COLUMNS: &str = "grid grid-cols-[1.5rem_minmax(0,1fr)_4.75rem] items-center gap-x-3 pl-3 md:grid-cols-[1.5rem_minmax(0,1fr)_6rem_9rem_4.75rem]";
pub const MUTED: &str = "text-slate-500 dark:text-slate-400";

#[component]
pub fn FileRow(
    volume: Segment,
    path: Segments,
    entry: Entry,
    index: usize,
    now: i64,
    has_menu: bool,
    menu_open: bool,
    selected: bool,
    /// Moves focus to the ⋯ button once it is rendered, then calls `on_focus_taken`.
    #[props(default)]
    take_focus: bool,
    #[props(default)] on_focus_taken: EventHandler<()>,
    on_select: EventHandler<(String, bool)>,
    on_menu: EventHandler<MenuRequest>,
) -> Element {
    let mut more = use_signal(|| None::<Rc<MountedData>>);
    use_effect(use_reactive!(|take_focus| {
        if take_focus && let Some(button) = more() {
            on_focus_taken.call(());
            spawn(async move {
                let _ = button.set_focus(true).await;
            });
        }
    }));
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
    let row_class = if selected {
        "bg-indigo-50 hover:bg-indigo-100/70 dark:bg-indigo-950/40 dark:hover:bg-indigo-950/60"
    } else if disabled {
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
            SelectCell {
                name: entry.name.clone(),
                checked: selected,
                on_select: {
                    let name = entry.name.clone();
                    move |range| on_select.call((name.clone(), range))
                },
            }
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
pub fn download_href(volume: &str, path: &Segments, kind: EntryKind) -> Option<String> {
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
