use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind};

use super::entry_actions::{
    EntryActionsLayer, select, selected_entries, toggle_all, use_entry_actions,
    use_retain_selection,
};
use super::file_row::MUTED;
use super::file_tile::FileTile;
use super::select_cell::SelectAll;
use super::selection_bar::SelectionBar;
use super::virtual_list::VirtualList;
use crate::actions::{RowAction, row_actions, selection_actions};
use crate::components::common::FocusTarget;
use crate::listing::{Sort, sorted};
use crate::routes::{Segment, Segments};
use crate::selection::Selection;

/// 9rem: the narrowest a tile gets.
const TILE_MIN_WIDTH: f64 = 144.0;
const GAP: f64 = 12.0;
/// What a tile adds below its square preview: borders, padding, two lines of name and the
/// detail line.
const TILE_TEXT_HEIGHT: f64 = 64.0;

/// How many tiles fit a row of `width` pixels, and the height of a row with its gap.
fn grid_layout(width: f64) -> (usize, f64) {
    // Before the first measure.
    if width <= 0.0 {
        return (1, TILE_MIN_WIDTH + TILE_TEXT_HEIGHT + GAP);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive width gives a small column count"
    )]
    let columns = (((width + GAP) / (TILE_MIN_WIDTH + GAP)).floor() as usize).max(1);
    #[expect(clippy::cast_precision_loss, reason = "a handful of columns")]
    let count = columns as f64;
    let tile = (width - (count - 1.0) * GAP) / count;
    (columns, tile + TILE_TEXT_HEIGHT + GAP)
}

#[component]
pub fn FileGrid(
    volume: Segment,
    path: Segments,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    read_only: bool,
    selection: Signal<Selection>,
) -> Element {
    let sort = use_context::<Signal<Sort>>();
    let actions = use_entry_actions();
    let select_all = use_signal(|| None::<Rc<MountedData>>);
    let mut width = use_signal(|| 0.0);
    let rows = use_memo(move || Rc::new(sorted(&entries.read(), sort())));
    let (columns, row_height) = grid_layout(width());
    let total = rows.read().len();
    let focus_pending = actions.focus_pending();
    let reveal = focus_pending
        .as_ref()
        .and_then(|name| rows.read().iter().position(|entry| &entry.name == name))
        .map(|index| index / columns);
    use_retain_selection(rows, selection);
    let chosen = selection();
    let selected = selected_entries(&rows.read(), &chosen);
    let bar_actions: Vec<RowAction> = {
        let kinds: Vec<EntryKind> = selected.iter().map(|(_, kind)| *kind).collect();
        selection_actions(&kinds, read_only)
    };
    let selected_count = selected.len();
    let menu_name = actions.menu_name();
    let (layer_volume, layer_path) = (volume.clone(), path.clone());

    let on_selection_action = {
        let (volume, path) = (volume.clone(), path.clone());
        let trigger = select_all().map(FocusTarget);
        move |action: RowAction| {
            actions.selection_action(action, volume.as_str(), &path, &selected, trigger.clone());
        }
    };
    let on_select =
        move |(name, range): (String, bool)| select(selection, &rows.read(), &name, range);

    rsx! {
        div {
            role: "region",
            aria_label: t!("table-label"),
            onkeydown: move |event| {
                if event.key() == Key::Escape && !selection.read().is_empty() {
                    selection.write().clear();
                }
            },
            class: "flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-slate-200 bg-white text-sm dark:border-slate-800 dark:bg-slate-900",
            div { class: "flex h-10 shrink-0 items-center gap-x-3 border-b border-slate-200 pl-3 text-xs dark:border-slate-800",
                SelectAll {
                    input: select_all,
                    checked: total > 0 && selected_count == total,
                    partial: selected_count > 0 && selected_count < total,
                    on_toggle: move |()| toggle_all(selection, &rows.read()),
                    role: "none",
                }
                if selected_count == 0 {
                    span { class: "font-medium {MUTED}", {t!("grid-count", count: total)} }
                } else {
                    SelectionBar {
                        count: selected_count,
                        actions: bar_actions,
                        on_action: on_selection_action,
                        on_clear: move |()| selection.write().clear(),
                        in_table: false,
                    }
                }
            }
            if total == 0 {
                p { class: "py-10 text-center {MUTED}", {t!("browse-empty")} }
            } else {
                VirtualList {
                    role: "list",
                    aria_label: t!("table-label"),
                    class: "min-h-0 flex-1 overflow-y-auto px-3 pt-3 [scrollbar-gutter:stable]",
                    total: total.div_ceil(columns),
                    row_height,
                    reveal,
                    on_width: move |measured: f64| {
                        if (measured - *width.peek()).abs() >= 1.0 {
                            width.set(measured);
                        }
                    },
                    row: move |row: usize| {
                        let start = row * columns;
                        let tiles: Vec<Entry> = rows.read().iter().skip(start).take(columns).cloned().collect();
                        rsx! {
                            div {
                                role: "none",
                                class: "grid gap-x-3 pb-3",
                                height: "{row_height}px",
                                grid_template_columns: "repeat({columns}, minmax(0, 1fr))",
                                for entry in tiles {
                                    FileTile {
                                        key: "{entry.name}",
                                        volume: volume.clone(),
                                        path: path.clone(),
                                        menu_open: menu_name.as_deref() == Some(entry.name.as_str()),
                                        selected: chosen.contains(&entry.name),
                                        selecting: !chosen.is_empty(),
                                        has_menu: !row_actions(entry.kind, read_only).is_empty(),
                                        take_focus: focus_pending.as_deref() == Some(entry.name.as_str()),
                                        on_focus_taken: move |()| actions.focus_taken(),
                                        on_select,
                                        on_menu: move |request| actions.open_menu(request),
                                        entry,
                                    }
                                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_follow_the_width() {
        assert_eq!(grid_layout(0.0), (1, 144.0 + 64.0 + 12.0));
        assert_eq!(grid_layout(100.0), (1, 100.0 + 64.0 + 12.0));
        assert_eq!(grid_layout(144.0).0, 1);
        assert_eq!(grid_layout(299.0).0, 1);
        // Two 144 px tiles and a gap.
        assert_eq!(grid_layout(300.0), (2, 144.0 + 64.0 + 12.0));
        // A phone: 375 px less the page and grid padding.
        assert_eq!(grid_layout(319.0).0, 2);
        assert_eq!(grid_layout(1000.0).0, 6);
    }

    #[test]
    fn tiles_share_the_row_width() {
        let (columns, height) = grid_layout(612.0);
        assert_eq!(columns, 4);
        assert!((height - (144.0 + 64.0 + 12.0)).abs() < f64::EPSILON);
        let (columns, height) = grid_layout(700.0);
        assert_eq!(columns, 4);
        assert!((height - (166.0 + 64.0 + 12.0)).abs() < f64::EPSILON);
    }
}
