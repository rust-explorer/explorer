use dioxus::prelude::*;

use super::uploads_panel::{destination, file_name};
use crate::components::common::ConflictPrompt;
use crate::uploads::use_uploads;

/// Asks what to do with the first upload whose destination already exists.
#[component]
pub fn ConflictDialog() -> Element {
    let uploads = use_uploads();
    let queue = uploads.queue();
    let queue = queue.read();
    let Some(item) = queue.pending_decision() else {
        return rsx! {};
    };
    let (volume, path) = destination(item);
    let folder = match path.rsplit_once('/') {
        Some((parent, _)) => format!("{volume}/{parent}"),
        None => volume.to_owned(),
    };

    let key = item.key;

    rsx! {
        // Keyed by item so the next conflict starts unchecked and takes focus again.
        ConflictPrompt {
            key: "{key.0}",
            name: file_name(path).to_owned(),
            folder,
            several: queue.batch_len(item.batch) > 1,
            allow_replace: true,
            on_decide: move |(decision, apply_to_all)| uploads.decide(key, decision, apply_to_all),
        }
    }
}
