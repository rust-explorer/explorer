use std::rc::Rc;

use dioxus::prelude::*;
use shared::api::browse::{Entry, EntryKind};
use shared::api::tasks::Location;

use super::file_row::download_href;
use super::row_menu::{MenuRequest, RowMenu};
use super::selection_bar::selection_href;
use crate::actions::RowAction;
use crate::api;
use crate::components::common::FocusTarget;
use crate::components::{DeleteDialog, RenameDialog, TransferDialog, TransferMode};
use crate::routes::{Segment, Segments};
use crate::selection::Selection;
use crate::tasks::{FolderSize, use_tasks};

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

/// The entry menu and the dialogs it opens, shared by the list and the grid. The view opens
/// menus through it and renders an [`EntryActionsLayer`] next to itself.
#[derive(Clone, Copy, PartialEq)]
pub struct EntryActions {
    menu: Signal<Option<MenuRequest>>,
    renaming: Signal<Option<Renaming>>,
    deleting: Signal<Option<Deleting>>,
    transferring: Signal<Option<Transferring>>,
    /// A renamed entry whose ⋯ takes focus once the refreshed listing shows it.
    focus_menu_of: Signal<Option<String>>,
}

pub fn use_entry_actions() -> EntryActions {
    EntryActions {
        menu: use_signal(|| None),
        renaming: use_signal(|| None),
        deleting: use_signal(|| None),
        transferring: use_signal(|| None),
        focus_menu_of: use_signal(|| None),
    }
}

impl EntryActions {
    pub fn open_menu(mut self, request: MenuRequest) {
        self.focus_menu_of.set(None);
        self.menu.set(Some(request));
    }

    /// The entry whose menu is open.
    pub fn menu_name(self) -> Option<String> {
        self.menu
            .read()
            .as_ref()
            .map(|open| open.entry.name.clone())
    }

    /// The entry whose ⋯ button should take focus.
    pub fn focus_pending(self) -> Option<String> {
        self.focus_menu_of.read().clone()
    }

    pub fn focus_taken(mut self) {
        self.focus_menu_of.set(None);
    }

    /// Runs a selection bar action on `selected`, entries of `dir`; `trigger` gets focus back
    /// once a dialog closes.
    pub fn selection_action(
        mut self,
        action: RowAction,
        volume: &str,
        dir: &Segments,
        selected: &[(String, EntryKind)],
        trigger: Option<FocusTarget>,
    ) {
        match action {
            RowAction::Download => {
                let chosen: Vec<(&str, EntryKind)> = selected
                    .iter()
                    .map(|(name, kind)| (name.as_str(), *kind))
                    .collect();
                if let Some(href) = selection_href(volume, dir, &chosen) {
                    api::navigate(&href);
                }
            }
            RowAction::Delete => self.deleting.set(Some(Deleting {
                names: selected.iter().map(|(name, _)| name.clone()).collect(),
                trigger,
                from_selection: true,
            })),
            RowAction::CopyTo | RowAction::MoveTo => self.transferring.set(Some(Transferring {
                mode: transfer_mode(action),
                entries: selected.to_vec(),
                trigger,
                from_selection: true,
            })),
            RowAction::Rename | RowAction::AnalyzeUsage | RowAction::CalculateSize => {}
        }
    }
}

/// Drops from the selection the names the listing no longer has.
pub fn use_retain_selection(rows: Memo<Rc<Vec<Entry>>>, mut selection: Signal<Selection>) {
    use_effect(move || {
        let rows = rows.read();
        let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
        selection.write().retain(&names);
    });
}

/// The selected entries, in listing order.
pub fn selected_entries(rows: &[Entry], selection: &Selection) -> Vec<(String, EntryKind)> {
    if selection.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .filter(|entry| selection.contains(&entry.name))
        .map(|entry| (entry.name.clone(), entry.kind))
        .collect()
}

/// Toggles `name`, or with `range` selects from the last toggled entry to it.
pub fn select(mut selection: Signal<Selection>, rows: &[Entry], name: &str, range: bool) {
    let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
    selection.with_mut(|selection| {
        if range {
            selection.select_range(&names, name);
        } else {
            selection.toggle(name);
        }
    });
}

/// Selects everything, or nothing when everything already is.
pub fn toggle_all(mut selection: Signal<Selection>, rows: &[Entry]) {
    let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
    let all = selection.read().len() == names.len();
    selection.with_mut(|selection| selection.set_all(&names, !all));
}

fn transfer_mode(action: RowAction) -> TransferMode {
    if action == RowAction::MoveTo {
        TransferMode::Move
    } else {
        TransferMode::Copy
    }
}

fn refocus(trigger: Option<FocusTarget>) {
    if let Some(FocusTarget(button)) = trigger {
        spawn(async move {
            let _ = button.set_focus(true).await;
        });
    }
}

/// The open entry menu and the rename, transfer and delete dialogs.
#[component]
pub fn EntryActionsLayer(
    actions: EntryActions,
    volume: Segment,
    path: Segments,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    read_only: bool,
    selection: Signal<Selection>,
) -> Element {
    let EntryActions {
        mut menu,
        mut renaming,
        mut deleting,
        mut transferring,
        mut focus_menu_of,
    } = actions;
    let (dialog_volume, dialog_parent) = (volume.as_str().to_owned(), path.joined());
    let transfer_parent = path.clone();
    let tasks = use_tasks();

    let on_action = move |action: RowAction| {
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
            RowAction::AnalyzeUsage | RowAction::CalculateSize => {
                let source = Location {
                    volume: volume.as_str().to_owned(),
                    path: path.child(&entry.name).joined(),
                };
                let open = action == RowAction::AnalyzeUsage;
                // Asking again for a size already known measures it anew.
                let known = matches!(
                    tasks
                        .list()
                        .read()
                        .folder_size(&source.volume, &source.path),
                    FolderSize::Known(_)
                );
                tasks.analyze(source, open, !open && known);
                if !open {
                    refocus(trigger);
                }
            }
        }
    };

    rsx! {
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
