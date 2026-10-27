use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::EntryKind;
use shared::api::conflict::ConflictPolicy;
use shared::api::tasks::{Location, TaskSpec};

use super::folder_picker::FolderPicker;
use crate::components::common::FocusTarget;
use crate::routes::Segments;
use crate::tasks::use_tasks;

/// Asks where to copy `entries` of `parent`, then starts one background task per entry.
#[component]
pub fn CopyDialog(
    volume: String,
    parent: Segments,
    entries: Vec<(String, EntryKind)>,
    on_close: EventHandler<()>,
    on_confirmed: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
    let tasks = use_tasks();
    let first = entries
        .first()
        .map(|(name, _)| name.clone())
        .unwrap_or_default();
    let title = t!("copy-title", count: entries.len(), name: first);
    let blocked: Vec<String> = entries
        .iter()
        .filter(|(_, kind)| *kind == EntryKind::Dir)
        .map(|(name, _)| parent.child(name).joined())
        .collect();
    let on_pick = {
        let (volume, parent) = (volume.clone(), parent.clone());
        move |(dest_volume, dest_path): (String, String)| {
            let dest_dir = Location {
                volume: dest_volume,
                path: dest_path,
            };
            tasks.start_transfers(copy_specs(&volume, &parent, &entries, &dest_dir));
            on_confirmed.call(());
        }
    };

    rsx! {
        FolderPicker {
            title,
            confirm_label: t!("picker-copy-here"),
            volume: volume.clone(),
            path: parent,
            blocked_volume: volume,
            blocked,
            on_pick,
            on_close,
            return_focus,
        }
    }
}

fn copy_specs(
    volume: &str,
    parent: &Segments,
    entries: &[(String, EntryKind)],
    dest_dir: &Location,
) -> Vec<(TaskSpec, EntryKind)> {
    entries
        .iter()
        .map(|(name, kind)| {
            let spec = TaskSpec::Copy {
                source: Location {
                    volume: volume.to_owned(),
                    path: parent.child(name).joined(),
                },
                dest_dir: dest_dir.clone(),
                conflict: ConflictPolicy::Fail,
            };
            (spec, *kind)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_copy_per_entry_into_the_picked_folder() {
        let parent = Segments::from(vec!["Films".to_owned()]);
        let dest_dir = Location {
            volume: "documents".to_owned(),
            path: "Backup".to_owned(),
        };
        let entries = [
            ("a b.mkv".to_owned(), EntryKind::File),
            ("Sub".to_owned(), EntryKind::Dir),
        ];
        let specs = copy_specs("media", &parent, &entries, &dest_dir);
        assert_eq!(
            specs[0],
            (
                TaskSpec::Copy {
                    source: Location {
                        volume: "media".to_owned(),
                        path: "Films/a b.mkv".to_owned(),
                    },
                    dest_dir: dest_dir.clone(),
                    conflict: ConflictPolicy::Fail,
                },
                EntryKind::File
            )
        );
        assert_eq!(specs[1].0.source().path, "Films/Sub");
        assert_eq!(specs[1].1, EntryKind::Dir);
    }
}
