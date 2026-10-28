use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};
use shared::api::thumbnails::has_thumbnail;
use unic_langid::LanguageIdentifier;

use super::file_row::{EntryIcon, MUTED};
use super::row_menu::{MenuAt, MenuRequest, ROW_MENU_ID, at_pointer, below_button};
use super::select_cell::SelectBox;
use crate::api;
use crate::components::common::FocusTarget;
use crate::components::common::icons::MoreIcon;
use crate::components::usage::FolderSizeLabel;
use crate::format::format_size;
use crate::i18n::attr;
use crate::routes::{Route, Segment, Segments};

const TILE: &str = "flex h-full flex-col overflow-hidden rounded-lg border p-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600";
const IDLE: &str = "border-slate-200 bg-white hover:border-slate-300 hover:bg-slate-50 dark:border-slate-800 dark:bg-slate-900 dark:hover:border-slate-700 dark:hover:bg-slate-800/60";
const CHOSEN: &str = "border-indigo-300 bg-indigo-50 hover:bg-indigo-100/70 dark:border-indigo-700 dark:bg-indigo-950/40 dark:hover:bg-indigo-950/60";
const DISABLED: &str = "border-slate-200 bg-white text-slate-400 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-600";
/// Hidden until hover or focus with a mouse; always shown on touch screens.
const ON_HOVER: &str = "pointer-fine:opacity-0 pointer-fine:group-hover:opacity-100 pointer-fine:group-focus-within:opacity-100";
const MORE: &str = "absolute right-1 top-1 inline-flex size-10 items-center justify-center rounded-md bg-white/90 text-slate-600 shadow-sm hover:bg-white hover:text-slate-900 focus-visible:outline-2 focus-visible:outline-indigo-600 pointer-fine:aria-expanded:opacity-100 dark:bg-slate-900/90 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-white";

/// The line under a tile's name.
pub fn tile_detail(entry: &Entry, locale: &LanguageIdentifier) -> String {
    match entry.kind {
        EntryKind::File => entry
            .size
            .map_or_else(|| "—".to_owned(), |size| format_size(size, locale)),
        EntryKind::Dir => t!("grid-folder"),
        EntryKind::Other => "—".to_owned(),
    }
}

/// An entry of the grid: a square preview, its name and [`tile_detail`], a checkbox and a ⋯
/// menu. `selecting` keeps the checkbox shown while anything is selected.
#[component]
pub fn FileTile(
    volume: Segment,
    path: Segments,
    entry: Entry,
    has_menu: bool,
    menu_open: bool,
    selected: bool,
    selecting: bool,
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
    let detail = tile_detail(&entry, &locale);
    let child = path.child(&entry.name);
    let disabled = entry.kind == EntryKind::Other;
    let look = if selected {
        CHOSEN
    } else if disabled {
        DISABLED
    } else {
        IDLE
    };
    let reveal = if selected || selecting { "" } else { ON_HOVER };

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
                open_at(at_pointer(&event));
            }
        }
    };
    let on_more = move |event: MouseEvent| {
        let open_at = open_at.clone();
        let button = more();
        spawn(async move {
            open_at(below_button(button, &event).await);
        });
    };

    let body = rsx! {
        span { class: "flex aspect-square w-full shrink-0 items-center justify-center overflow-hidden rounded-md bg-slate-100 dark:bg-slate-800",
            TilePreview { volume: volume.as_str().to_owned(), path: child.joined(), entry: entry.clone() }
        }
        span { class: "mt-2 line-clamp-2 h-10 break-words text-sm leading-5", {entry.name.clone()} }
        span { class: "h-4 truncate text-xs leading-4 {MUTED}",
            if entry.kind == EntryKind::Dir {
                FolderSizeLabel { volume: volume.as_str().to_owned(), path: child.joined(), unknown: detail }
            } else {
                {detail}
            }
        }
    };

    rsx! {
        div {
            role: "listitem",
            class: "group relative h-full min-w-0",
            title: if disabled { t!("entry-unavailable") } else { entry.name.clone() },
            oncontextmenu: on_context_menu,
            match entry.kind {
                EntryKind::Dir => rsx! {
                    Link {
                        to: Route::Browse { volume: volume.clone(), path: child.clone() },
                        class: "{TILE} {look}",
                        {body}
                    }
                },
                EntryKind::File => rsx! {
                    a {
                        href: api::download_url(volume.as_str(), &child.joined()),
                        download: "",
                        class: "{TILE} {look}",
                        {body}
                    }
                },
                EntryKind::Other => rsx! {
                    div { aria_disabled: "true", class: "{TILE} {look}",
                        {body}
                        span { class: "sr-only", {t!("entry-unavailable")} }
                    }
                },
            }
            label { class: "absolute left-1 top-1 flex size-10 cursor-pointer items-center justify-center {reveal}",
                SelectBox {
                    name: entry.name.clone(),
                    checked: selected,
                    on_select: {
                        let name = entry.name.clone();
                        move |range| on_select.call((name.clone(), range))
                    },
                }
            }
            if has_menu {
                button {
                    r#type: "button",
                    class: "{MORE} {ON_HOVER}",
                    aria_label: attr(t!("row-menu-open", name: entry.name.clone())),
                    aria_haspopup: "menu",
                    aria_expanded: menu_open,
                    aria_controls: if menu_open { ROW_MENU_ID },
                    onmounted: move |event| more.set(Some(event.data())),
                    onclick: on_more,
                    MoreIcon { class: "size-5" }
                }
            }
        }
    }
}

/// A thumbnail for the images the server renders, falling back to the type icon.
#[component]
fn TilePreview(volume: String, path: String, entry: Entry) -> Element {
    let mut failed = use_signal(|| None::<String>);
    let url = (entry.kind == EntryKind::File && has_thumbnail(&entry.name))
        .then(|| api::thumbnail_url(&volume, &path, entry.modified_ms))
        .filter(|url| failed.read().as_ref() != Some(url));

    match url {
        Some(url) => rsx! {
            img {
                src: url.clone(),
                alt: "",
                "loading": "lazy",
                "decoding": "async",
                class: "size-full object-cover",
                onerror: move |_| failed.set(Some(url.clone())),
            }
        },
        None => rsx! {
            EntryIcon { kind: entry.kind, name: entry.name, size: "size-12" }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_tiles_show_their_size() {
        let entry = Entry {
            name: "a.jpg".to_owned(),
            kind: EntryKind::File,
            size: Some(2048),
            modified_ms: None,
        };
        let locale: LanguageIdentifier = "en-US".parse().unwrap();
        assert_eq!(tile_detail(&entry, &locale), format_size(2048, &locale));
        let unknown = Entry {
            size: None,
            ..entry
        };
        assert_eq!(tile_detail(&unknown, &locale), "—");
    }
}
