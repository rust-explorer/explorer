use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};

use super::file_row::{COLUMNS, FileRow, MUTED, download_href};
use super::row_menu::{MenuRequest, RowMenu, menu_actions};
use super::select_cell::SelectAll;
use super::selection_bar::{SelectionBar, selection_href};
use super::virtual_list::VirtualList;
use crate::actions::{RowAction, selection_actions};
use crate::api;
use crate::components::common::FocusTarget;
use crate::components::common::icons::{ArrowDownIcon, ArrowUpIcon};
use crate::components::{DeleteDialog, RenameDialog, TransferDialog, TransferMode};
use crate::format::now_ms;
use crate::listing::{Sort, SortKey, sorted};
use crate::routes::{Segment, Segments};
use crate::selection::Selection;

const ROW_HEIGHT: f64 = 44.0;

/// A rename in progress and the ⋯ button that gets focus back.
#[derive(Clone, PartialEq)]
struct Renaming {
    entry: Entry,
    trigger: Option<FocusTarget>,
}

/// Entries waiting for the delete confirmation; from the selection bar, a confirmed delete
/// clears the selection.
#[derive(Clone, PartialEq)]
struct Deleting {
    names: Vec<String>,
    trigger: Option<FocusTarget>,
    from_selection: bool,
}

/// Entries waiting for a destination folder; from the selection bar, a confirmed copy or move
/// clears the selection.
#[derive(Clone, PartialEq)]
struct Transferring {
    mode: TransferMode,
    entries: Vec<(String, EntryKind)>,
    trigger: Option<FocusTarget>,
    from_selection: bool,
}

#[component]
pub fn FileTable(
    volume: Segment,
    path: Segments,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    read_only: bool,
    selection: Signal<Selection>,
) -> Element {
    let mut sort = use_context::<Signal<Sort>>();
    let mut menu = use_signal(|| None::<MenuRequest>);
    let mut renaming = use_signal(|| None::<Renaming>);
    let mut deleting = use_signal(|| None::<Deleting>);
    let mut transferring = use_signal(|| None::<Transferring>);
    // A renamed entry whose ⋯ takes focus once the refreshed listing shows it.
    let mut focus_menu_of = use_signal(|| None::<String>);
    let select_all = use_signal(|| None::<Rc<MountedData>>);
    let rows = use_memo(move || Rc::new(sorted(&entries.read(), sort())));
    let now = use_memo(move || {
        let _ = entries.read();
        now_ms()
    });
    let total = rows.read().len();
    let reveal = focus_menu_of
        .read()
        .as_ref()
        .and_then(|name| rows.read().iter().position(|entry| &entry.name == name));
    use_effect(move || {
        let rows = rows.read();
        let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
        selection.write().retain(&names);
    });
    let chosen = selection();
    let selected: Vec<(String, EntryKind)> = if chosen.is_empty() {
        Vec::new()
    } else {
        rows.read()
            .iter()
            .filter(|entry| chosen.contains(&entry.name))
            .map(|entry| (entry.name.clone(), entry.kind))
            .collect()
    };
    let bar_actions: Vec<RowAction> = {
        let kinds: Vec<EntryKind> = selected.iter().map(|(_, kind)| *kind).collect();
        selection_actions(&kinds, read_only)
    };
    let selected_count = selected.len();
    let on_sort = move |column: SortKey| sort.set(sort().toggled(column));
    let menu_name = menu.read().as_ref().map(|open| open.entry.name.clone());
    let (dialog_volume, dialog_parent) = (volume.as_str().to_owned(), path.joined());
    let transfer_parent = path.clone();

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
                RowAction::Delete => deleting.set(Some(Deleting {
                    names: vec![entry.name],
                    trigger,
                    from_selection: false,
                })),
                RowAction::Download => {
                    if let Some(href) =
                        download_href(volume.as_str(), &path.child(&entry.name), entry.kind)
                    {
                        api::navigate(&href);
                    }
                    refocus(trigger);
                }
                RowAction::CopyTo | RowAction::MoveTo => transferring.set(Some(Transferring {
                    mode: transfer_mode(action),
                    entries: vec![(entry.name, entry.kind)],
                    trigger,
                    from_selection: false,
                })),
            }
        }
    };

    let on_selection_action = {
        let (volume, path) = (volume.clone(), path.clone());
        // The bar goes away on confirm: focus goes back to the select-all box.
        let trigger = select_all().map(FocusTarget);
        move |action: RowAction| match action {
            RowAction::Download => {
                let chosen: Vec<(&str, EntryKind)> = selected
                    .iter()
                    .map(|(name, kind)| (name.as_str(), *kind))
                    .collect();
                if let Some(href) = selection_href(volume.as_str(), &path, &chosen) {
                    api::navigate(&href);
                }
            }
            RowAction::Delete => deleting.set(Some(Deleting {
                names: selected.iter().map(|(name, _)| name.clone()).collect(),
                trigger: trigger.clone(),
                from_selection: true,
            })),
            RowAction::CopyTo | RowAction::MoveTo => transferring.set(Some(Transferring {
                mode: transfer_mode(action),
                entries: selected.clone(),
                trigger: trigger.clone(),
                from_selection: true,
            })),
            RowAction::Rename => {}
        }
    };
    let on_select = move |(name, range): (String, bool)| {
        let rows = rows.read();
        let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
        selection.with_mut(|selection| {
            if range {
                selection.select_range(&names, &name);
            } else {
                selection.toggle(&name);
            }
        });
    };
    let on_select_all = move |()| {
        let rows = rows.read();
        let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
        let all = selection.read().len() == names.len();
        selection.with_mut(|selection| selection.set_all(&names, !all));
    };

    rsx! {
        div {
            role: "table",
            aria_label: t!("table-label"),
            aria_rowcount: total + 1,
            onkeydown: move |event| {
                if event.key() == Key::Escape && !selection.read().is_empty() {
                    selection.write().clear();
                }
            },
            class: "flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-slate-200 bg-white text-sm dark:border-slate-800 dark:bg-slate-900",
            div { role: "rowgroup", class: "shrink-0 overflow-hidden border-b border-slate-200 [scrollbar-gutter:stable] dark:border-slate-800",
                div {
                    role: "row",
                    aria_rowindex: 1,
                    class: if selected_count == 0 { "{COLUMNS} h-9 text-xs font-medium {MUTED}" } else { "flex h-9 items-center gap-x-3 pl-3 text-xs" },
                    SelectAll {
                        input: select_all,
                        checked: total > 0 && selected_count == total,
                        partial: selected_count > 0 && selected_count < total,
                        on_toggle: on_select_all,
                    }
                    if selected_count == 0 {
                        HeaderCell { column: SortKey::Name, label: t!("table-name"), sort: sort(), on_sort }
                        HeaderCell { column: SortKey::Size, label: t!("table-size"), sort: sort(), on_sort }
                        HeaderCell { column: SortKey::Modified, label: t!("table-modified"), sort: sort(), on_sort }
                        div { role: "columnheader", span { class: "sr-only", {t!("table-actions")} } }
                    } else {
                        SelectionBar {
                            count: selected_count,
                            actions: bar_actions,
                            on_action: on_selection_action,
                            on_clear: move |()| selection.write().clear(),
                        }
                    }
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
                    reveal,
                    row: move |index: usize| {
                        let entry = rows.read()[index].clone();
                        let menu_open = menu_name.as_deref() == Some(entry.name.as_str());
                        let is_selected = chosen.contains(&entry.name);
                        let has_menu = !menu_actions(&entry, read_only).is_empty();
                        let take_focus = focus_menu_of.read().as_deref() == Some(entry.name.as_str());
                        rsx! {
                            FileRow {
                                volume: volume.clone(),
                                path: path.clone(),
                                entry,
                                index,
                                now: now(),
                                has_menu,
                                menu_open,
                                selected: is_selected,
                                take_focus,
                                on_focus_taken: move |()| focus_menu_of.set(None),
                                on_select,
                                on_menu: move |request| {
                                    focus_menu_of.set(None);
                                    menu.set(Some(request));
                                },
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
                on_close: move |restore_focus: bool| {
                    if let Some(open) = menu.take()
                        && restore_focus
                    {
                        refocus(open.trigger);
                    }
                },
                on_action,
            }
        }
        if let Some(Renaming { entry, trigger }) = renaming() {
            RenameDialog {
                volume: dialog_volume.clone(),
                parent: dialog_parent.clone(),
                entry,
                entries,
                return_focus: trigger,
                on_close: move |()| renaming.set(None),
                on_renamed: move |name| focus_menu_of.set(Some(name)),
            }
        }
        if let Some(Transferring { mode, entries, trigger, from_selection }) = transferring() {
            TransferDialog {
                mode,
                volume: dialog_volume.clone(),
                parent: transfer_parent.clone(),
                entries,
                return_focus: trigger,
                on_close: move |()| transferring.set(None),
                on_confirmed: move |()| {
                    transferring.set(None);
                    if from_selection {
                        selection.write().clear();
                    }
                },
            }
        }
        if let Some(Deleting { names, trigger, from_selection }) = deleting() {
            DeleteDialog {
                volume: dialog_volume,
                parent: dialog_parent,
                names,
                return_focus: trigger,
                on_close: move |()| deleting.set(None),
                on_confirmed: move |()| {
                    deleting.set(None);
                    if from_selection {
                        selection.write().clear();
                    }
                },
            }
        }
    }
}

fn transfer_mode(action: RowAction) -> TransferMode {
    if action == RowAction::MoveTo {
        TransferMode::Move
    } else {
        TransferMode::Copy
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
