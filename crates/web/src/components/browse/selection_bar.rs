use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::EntryKind;
use shared::api::downloads::ZipSelection;

use super::row_menu::action_label;
use crate::actions::RowAction;
use crate::api;
use crate::components::common::icons::{CloseIcon, CopyIcon, DownloadIcon, MoveIcon, TrashIcon};
use crate::routes::Segments;

/// Name, size, modified and actions.
const COLUMNS_AFTER_SELECT: usize = 4;
const BUTTON: &str = "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-indigo-600 dark:text-slate-200 dark:hover:bg-slate-800";

/// Replaces the column headers while entries are selected, as one header spanning the
/// columns after the select-all box. Outside a table (`in_table: false`), a plain group.
#[component]
pub fn SelectionBar(
    count: usize,
    actions: Vec<RowAction>,
    on_action: EventHandler<RowAction>,
    on_clear: EventHandler<()>,
    #[props(default = true)] in_table: bool,
) -> Element {
    rsx! {
        div {
            role: if in_table { "columnheader" } else { "group" },
            aria_colspan: if in_table { COLUMNS_AFTER_SELECT },
            class: "flex min-w-0 flex-1 items-center gap-1 pr-2",
            span { role: "status", class: "mr-2 truncate text-sm font-medium text-slate-900 dark:text-white",
                {t!("selection-count", count: count)}
            }
            for action in actions {
                button {
                    key: "{action:?}",
                    r#type: "button",
                    class: "{BUTTON} {danger(action)}",
                    onclick: move |_| on_action.call(action),
                    ActionIcon { action }
                    span { class: "hidden sm:inline", {action_label(action)} }
                    span { class: "sr-only sm:hidden", {action_label(action)} }
                }
            }
            button {
                r#type: "button",
                class: "{BUTTON} ml-auto",
                onclick: move |_| on_clear.call(()),
                CloseIcon {}
                span { class: "hidden sm:inline", {t!("selection-clear")} }
                span { class: "sr-only sm:hidden", {t!("selection-clear")} }
            }
        }
    }
}

fn danger(action: RowAction) -> &'static str {
    if action == RowAction::Delete {
        "!text-red-700 dark:!text-red-400"
    } else {
        ""
    }
}

#[component]
fn ActionIcon(action: RowAction) -> Element {
    match action {
        RowAction::Download => rsx! { DownloadIcon {} },
        RowAction::MoveTo => rsx! { MoveIcon {} },
        RowAction::CopyTo => rsx! { CopyIcon {} },
        RowAction::Delete => rsx! { TrashIcon {} },
        RowAction::Rename | RowAction::AnalyzeUsage | RowAction::CalculateSize => rsx! {},
    }
}

/// How a selection downloads: one entry from its URL, several as one zip of `dir`, whose names
/// are stored first since thousands of them don't fit in a URL.
#[derive(Debug, PartialEq, Eq)]
pub enum SelectionDownload {
    Href(String),
    Zip(ZipSelection),
}

/// One file downloads as is, one folder as a zip, several entries as one zip of `dir`; an entry
/// that is neither a file nor a folder can't be downloaded.
pub fn selection_download(
    volume: &str,
    dir: &Segments,
    selected: &[(&str, EntryKind)],
) -> Option<SelectionDownload> {
    if selected.iter().any(|(_, kind)| *kind == EntryKind::Other) {
        return None;
    }
    match selected {
        [] => None,
        [(name, EntryKind::File)] => Some(SelectionDownload::Href(api::download_url(
            volume,
            &dir.child(name).joined(),
        ))),
        [(name, _)] => Some(SelectionDownload::Href(api::zip_url(
            volume,
            &dir.child(name).joined(),
        ))),
        _ => Some(SelectionDownload::Zip(ZipSelection {
            volume: volume.to_owned(),
            path: dir.joined(),
            names: selected
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EntryKind::{Dir, File, Other};

    #[test]
    fn download_by_selection() {
        let dir = Segments::from(vec!["Films".to_owned()]);
        let download = |selected: &[(&str, EntryKind)]| selection_download("media", &dir, selected);
        let href = |url: &str| Some(SelectionDownload::Href(url.to_owned()));
        assert_eq!(download(&[]), None);
        assert_eq!(
            download(&[("a b.mkv", File)]),
            href("/files/download?volume=media&path=Films%2Fa%20b.mkv")
        );
        assert_eq!(
            download(&[("Sub", Dir)]),
            href("/files/zip?volume=media&path=Films%2FSub")
        );
        assert_eq!(
            download(&[("a", File), ("Sub", Dir)]),
            Some(SelectionDownload::Zip(ZipSelection {
                volume: "media".to_owned(),
                path: "Films".to_owned(),
                names: vec!["a".to_owned(), "Sub".to_owned()],
            }))
        );
        assert_eq!(download(&[("a", File), ("fifo", Other)]), None);
    }
}
