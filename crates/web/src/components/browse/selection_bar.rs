use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::EntryKind;

use super::row_menu::action_label;
use crate::actions::RowAction;
use crate::api;
use crate::components::common::icons::{CloseIcon, CopyIcon, DownloadIcon, MoveIcon, TrashIcon};
use crate::routes::Segments;

const BUTTON: &str = "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-indigo-600 dark:text-slate-200 dark:hover:bg-slate-800";

/// Replaces the column headers while entries are selected.
#[component]
pub fn SelectionBar(
    count: usize,
    actions: Vec<RowAction>,
    on_action: EventHandler<RowAction>,
    on_clear: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "flex min-w-0 flex-1 items-center gap-1 pr-2",
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
        RowAction::Rename => rsx! {},
    }
}

/// One file downloads as is, one folder as a zip, several entries as one zip of `dir`; an entry
/// that is neither a file nor a folder can't be downloaded.
pub fn selection_href(
    volume: &str,
    dir: &Segments,
    selected: &[(&str, EntryKind)],
) -> Option<String> {
    if selected.iter().any(|(_, kind)| *kind == EntryKind::Other) {
        return None;
    }
    match selected {
        [] => None,
        [(name, EntryKind::File)] => Some(api::download_url(volume, &dir.child(name).joined())),
        [(name, _)] => Some(api::zip_url(volume, &dir.child(name).joined())),
        _ => {
            let names: Vec<&str> = selected.iter().map(|(name, _)| *name).collect();
            Some(api::zip_selection_url(volume, &dir.joined(), &names))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EntryKind::{Dir, File, Other};

    #[test]
    fn download_href_by_selection() {
        let dir = Segments::from(vec!["Films".to_owned()]);
        let href = |selected: &[(&str, EntryKind)]| selection_href("media", &dir, selected);
        assert_eq!(href(&[]), None);
        assert_eq!(
            href(&[("a b.mkv", File)]).as_deref(),
            Some("/files/download?volume=media&path=Films%2Fa%20b.mkv")
        );
        assert_eq!(
            href(&[("Sub", Dir)]).as_deref(),
            Some("/files/zip?volume=media&path=Films%2FSub")
        );
        assert_eq!(
            href(&[("a", File), ("Sub", Dir)]).as_deref(),
            Some("/files/zip?volume=media&path=Films&name=a&name=Sub")
        );
        assert_eq!(href(&[("a", File), ("fifo", Other)]), None);
    }
}
