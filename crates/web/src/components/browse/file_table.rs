use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};

use super::empty_folder::EmptyFolder;
use super::entry_actions::{
    EntryActionsLayer, select, selected_entries, toggle_all, use_entry_actions,
    use_retain_selection,
};
use super::file_row::{COLUMNS, FileRow, MUTED};
use super::select_cell::SelectAll;
use super::selection_bar::SelectionBar;
use super::virtual_list::VirtualList;
use crate::actions::{RowAction, row_actions, selection_actions};
use crate::components::common::FocusTarget;
use crate::components::common::icons::{ArrowDownIcon, ArrowUpIcon};
use crate::format::now_ms;
use crate::listing::{Sort, SortKey, sorted};
use crate::routes::{Segment, Segments};
use crate::selection::Selection;

const ROW_HEIGHT: f64 = 44.0;

#[component]
pub fn FileTable(
    volume: Segment,
    path: Segments,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    read_only: bool,
    selection: Signal<Selection>,
) -> Element {
    let mut sort = use_context::<Signal<Sort>>();
    let actions = use_entry_actions();
    let select_all = use_signal(|| None::<Rc<MountedData>>);
    let rows = use_memo(move || Rc::new(sorted(&entries.read(), sort())));
    let now = use_memo(move || {
        let _ = entries.read();
        now_ms()
    });
    let total = rows.read().len();
    let focus_pending = actions.focus_pending();
    let reveal = focus_pending
        .as_ref()
        .and_then(|name| rows.read().iter().position(|entry| &entry.name == name));
    use_retain_selection(rows, selection);
    let chosen = selection();
    let selected = selected_entries(&rows.read(), &chosen);
    let bar_actions: Vec<RowAction> = {
        let kinds: Vec<EntryKind> = selected.iter().map(|(_, kind)| *kind).collect();
        selection_actions(&kinds, read_only)
    };
    let selected_count = selected.len();
    let on_sort = move |column: SortKey| sort.set(sort().toggled(column));
    let menu_name = actions.menu_name();
    let (layer_volume, layer_path) = (volume.clone(), path.clone());

    let on_selection_action = {
        let (volume, path) = (volume.clone(), path.clone());
        // The bar goes away on confirm: focus goes back to the select-all box.
        let trigger = select_all().map(FocusTarget);
        move |action: RowAction| {
            actions.selection_action(action, volume.as_str(), &path, &selected, trigger.clone());
        }
    };
    let on_select =
        move |(name, range): (String, bool)| select(selection, &rows.read(), &name, range);
    let on_select_all = move |()| toggle_all(selection, &rows.read());

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
                EmptyFolder { read_only }
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
                        let has_menu = !row_actions(entry.kind, read_only).is_empty();
                        let take_focus = focus_pending.as_deref() == Some(entry.name.as_str());
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
                                on_focus_taken: move |()| actions.focus_taken(),
                                on_select,
                                on_menu: move |request| actions.open_menu(request),
                            }
                        }
                    },
                }
            }
        }
        EntryActionsLayer {
            actions,
            volume: layer_volume,
            path: layer_path,
            entries,
            read_only,
            selection,
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
