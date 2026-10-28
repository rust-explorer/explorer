use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind, Listing, VolumeInfo};

use super::form::{BUTTON, PRIMARY};
use crate::api::{self, ApiFailure};
use crate::components::VirtualList;
use crate::components::common::icons::FolderIcon;
use crate::components::common::{FocusTarget, Modal, error_message};
use crate::listing::{Sort, sorted};
use crate::routes::Segments;

const TITLE_ID: &str = "folder-picker-title";
const VOLUME_ID: &str = "folder-picker-volume";
const CRUMB: &str = "truncate rounded hover:text-indigo-600 hover:underline focus-visible:outline-2 focus-visible:outline-indigo-600 dark:hover:text-indigo-400";
const CURRENT: &str = "truncate font-semibold text-gray-900 dark:text-white";
const FOLDER: &str = "flex w-full items-center gap-2 h-full rounded-md px-2 text-left text-sm text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:text-slate-400 disabled:hover:bg-transparent dark:text-slate-200 dark:hover:bg-slate-800 dark:disabled:text-slate-600";
/// The fixed height of a folder row, for the virtual list.
const FOLDER_HEIGHT: f64 = 32.0;
const STATUS: &str = "text-sm text-slate-500 dark:text-slate-400";
const SELECT: &str = "mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-900 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:bg-slate-950 dark:text-white";

/// The folder being shown.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    volume: String,
    path: Segments,
}

/// Picks a destination folder, starting at `volume`/`path`, among the folders of the writable
/// volumes (read-only ones are listed, disabled). The `blocked` folders of `blocked_volume` and everything under them can't be picked,
/// nor, with `start_unpickable`, the starting folder itself (a move to where the entries are).
#[component]
pub fn FolderPicker(
    title: String,
    confirm_label: String,
    volume: String,
    path: Segments,
    blocked_volume: String,
    blocked: Vec<String>,
    #[props(default)] start_unpickable: bool,
    on_pick: EventHandler<(String, String)>,
    on_close: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
    let start = use_hook(|| Shown {
        volume: volume.clone(),
        path: path.clone(),
    });
    let mut shown = use_signal(|| Shown { volume, path });
    let mut volumes = use_resource(|| api::get_json::<Vec<VolumeInfo>>("/api/volumes"));
    let url = use_memo(move || {
        let shown = shown.read();
        api::entries_url(&shown.volume, &shown.path.joined())
    });
    let mut listing = use_resource(move || {
        let url = url();
        async move {
            let result = api::get_json::<Listing>(&url).await;
            (url, result.map(|listing| folders(&listing.entries)))
        }
    });

    let volumes_state = volumes
        .read()
        .as_ref()
        .map(|result| result.as_ref().err().cloned());
    let volumes_ready = volumes_state == Some(None);
    let all_volumes: Vec<VolumeInfo> = match &*volumes.read() {
        Some(Ok(volumes)) => volumes.clone(),
        _ => Vec::new(),
    };
    let writable: Vec<String> = all_volumes
        .iter()
        .filter(|info| !info.read_only)
        .map(|info| info.name.clone())
        .collect();
    let current = shown();
    let blocked_here = |path: &str| is_blocked(&current.volume, path, &blocked_volume, &blocked);
    let can_pick = writable.contains(&current.volume)
        && !blocked_here(&current.path.joined())
        && !(start_unpickable && current == start);
    let folders = match &*listing.read() {
        Some((fetched, result)) if *fetched == url() => Some(result.clone()),
        _ => None,
    };
    let names = current.path.names().to_vec();

    rsx! {
        Modal { labelled_by: TITLE_ID, on_close: move |()| on_close.call(()), return_focus,
            h2 {
                id: TITLE_ID,
                class: "break-words text-sm font-medium text-gray-900 dark:text-white",
                {title}
            }
            label {
                // The select only exists once the volumes are in.
                r#for: if volumes_ready { VOLUME_ID },
                class: "mt-4 block text-sm text-slate-700 dark:text-slate-300",
                {t!("picker-volume")}
            }
            match volumes_state {
                None => rsx! {
                    p { class: "{STATUS} mt-1", {t!("loading")} }
                },
                Some(Some(failure)) => rsx! {
                    LoadFailed { failure, on_retry: move |()| volumes.restart() }
                },
                Some(None) => rsx! {
                    select {
                        id: VOLUME_ID,
                        class: SELECT,
                        autofocus: true,
                        onchange: move |event| shown.set(Shown {
                            volume: event.value(),
                            path: Segments::default(),
                        }),
                        if !writable.contains(&current.volume) {
                            option { value: "", disabled: true, selected: true, {t!("picker-choose-volume")} }
                        }
                        for info in all_volumes.iter() {
                            option {
                                key: "{info.name}",
                                value: "{info.name}",
                                disabled: info.read_only,
                                selected: !info.read_only && info.name == current.volume,
                                if info.read_only {
                                    {t!("picker-volume-read-only", name: info.name.clone())}
                                } else {
                                    {info.name.clone()}
                                }
                            }
                        }
                    }
                },
            }
            nav { aria_label: t!("picker-location"), class: "mt-4",
                ol { class: "flex min-w-0 flex-wrap items-center gap-x-1.5 text-sm text-slate-500 dark:text-slate-400",
                    li { class: "flex min-w-0 items-center",
                        if names.is_empty() {
                            span { class: CURRENT, aria_current: "location", {current.volume.clone()} }
                        } else {
                            button {
                                r#type: "button",
                                class: CRUMB,
                                onclick: move |_| shown.write().path = Segments::default(),
                                {current.volume.clone()}
                            }
                        }
                    }
                    for (index, name) in names.iter().cloned().enumerate() {
                        li { key: "{index}", class: "flex min-w-0 items-center gap-x-1.5",
                            span { aria_hidden: "true", "›" }
                            if index + 1 == names.len() {
                                span { class: CURRENT, aria_current: "location", {name} }
                            } else {
                                button {
                                    r#type: "button",
                                    class: CRUMB,
                                    onclick: move |_| {
                                        let prefix = shown.read().path.prefix(index + 1);
                                        shown.write().path = prefix;
                                    },
                                    {name}
                                }
                            }
                        }
                    }
                }
            }
            div { class: "mt-2 flex h-56 flex-col rounded-md border border-slate-200 p-1 dark:border-slate-800",
                {match folders {
                    Some(Ok(folders)) if folders.is_empty() => rsx! {
                        p { class: "p-2 {STATUS}", {t!("picker-empty")} }
                    },
                    Some(Ok(folders)) => {
                        let total = folders.len();
                        let folders = Rc::new(folders);
                        let blocked_volume = blocked_volume.clone();
                        let blocked = blocked.clone();
                        let current = current.clone();
                        rsx! {
                            VirtualList {
                                role: "list",
                                aria_label: t!("picker-folders"),
                                class: "min-h-0 flex-1 overflow-y-auto",
                                total,
                                row_height: FOLDER_HEIGHT,
                                row: move |index: usize| {
                                    let name = folders[index].clone();
                                    let child = current.path.child(&name);
                                    let disabled = is_blocked(&current.volume, &child.joined(), &blocked_volume, &blocked);
                                    rsx! {
                                        div { role: "listitem", height: "{FOLDER_HEIGHT}px",
                                            button {
                                                r#type: "button",
                                                class: FOLDER,
                                                disabled,
                                                onclick: move |_| shown.write().path = child.clone(),
                                                FolderIcon { class: "size-4 shrink-0 text-indigo-500" }
                                                span { class: "truncate", {name} }
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    }
                    Some(Err(failure)) => rsx! {
                        LoadFailed { failure, on_retry: move |()| listing.restart() }
                    },
                    None => rsx! {
                        p { class: "p-2 {STATUS}", {t!("loading")} }
                    },
                }}
            }
            div { class: "mt-5 flex justify-end gap-3",
                button { r#type: "button", class: BUTTON, onclick: move |_| on_close.call(()), {t!("cancel")} }
                button {
                    r#type: "button",
                    class: PRIMARY,
                    disabled: !can_pick,
                    onclick: move |_| {
                        let shown = shown.read();
                        on_pick.call((shown.volume.clone(), shown.path.joined()));
                    },
                    {confirm_label}
                }
            }
        }
    }
}

#[component]
fn LoadFailed(failure: ApiFailure, on_retry: EventHandler<()>) -> Element {
    rsx! {
        div { role: "alert", class: "mt-1 flex items-center gap-3 p-2 text-sm text-slate-600 dark:text-slate-400",
            span { {error_message(&failure)} }
            button {
                r#type: "button",
                class: BUTTON,
                onclick: move |_| on_retry.call(()),
                {t!("try-again")}
            }
        }
    }
}

/// The folder names among `entries`, in the browse order.
fn folders(entries: &[Entry]) -> Vec<String> {
    sorted(entries, Sort::default())
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::Dir)
        .map(|entry| entry.name)
        .collect()
}

/// Whether `path` of `volume` is one of the `blocked` folders of `blocked_volume` or under one.
fn is_blocked(volume: &str, path: &str, blocked_volume: &str, blocked: &[String]) -> bool {
    volume == blocked_volume
        && blocked.iter().any(|folder| {
            path == folder
                || path
                    .strip_prefix(folder.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            size: None,
            modified_ms: None,
        }
    }

    #[test]
    fn only_folders_are_listed() {
        let entries = [
            entry("b", EntryKind::Dir),
            entry("a.txt", EntryKind::File),
            entry("A", EntryKind::Dir),
            entry("fifo", EntryKind::Other),
        ];
        assert_eq!(folders(&entries), ["A", "b"]);
    }

    #[test]
    fn sources_and_their_descendants_are_blocked() {
        let blocked = ["Films/F".to_owned(), "G".to_owned()];
        let check = |volume, path| is_blocked(volume, path, "media", &blocked);
        for path in ["Films/F", "Films/F/sub", "G", "G/a/b"] {
            assert!(check("media", path), "{path}");
        }
        for path in ["", "Films", "Films/Fx", "Gx", "Films/G"] {
            assert!(!check("media", path), "{path}");
        }
        assert!(!check("documents", "G"));
    }
}
