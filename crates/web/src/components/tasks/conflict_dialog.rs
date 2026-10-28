use dioxus::prelude::*;

use crate::components::common::{ConflictPrompt, file_name};
use crate::tasks::use_tasks;

/// Asks what to do with the first copy or move whose destination already exists.
#[component]
pub fn TaskConflictDialog() -> Element {
    let tasks = use_tasks();
    let list = tasks.list();
    let list = list.read();
    let Some(task) = list.pending_decision() else {
        return rsx! {};
    };
    let Some(conflict) = task.conflict() else {
        return rsx! {};
    };
    let folder = match conflict.path.rsplit_once('/') {
        Some((parent, _)) => format!("{}/{parent}", conflict.volume),
        None => conflict.volume.clone(),
    };
    let key = task.key;

    rsx! {
        // Keyed by task so the next conflict starts unchecked and takes focus again.
        ConflictPrompt {
            key: "task-{key.0}",
            name: file_name(&conflict.path).to_owned(),
            folder,
            several: list.batch_len(task.batch) > 1,
            allow_replace: task.allows_replace(),
            on_decide: move |(decision, apply_to_all)| tasks.decide(key, decision, apply_to_all),
        }
    }
}
