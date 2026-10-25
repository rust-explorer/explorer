use shared::api::uploads::ConflictPolicy;

use super::{Decision, ItemKey};
use crate::api::ApiFailure;
use crate::uploads::plan::FileSpec;

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    CreateFolder {
        key: ItemKey,
        volume: String,
        path: String,
    },
    Create {
        key: ItemKey,
        spec: FileSpec,
        conflict: ConflictPolicy,
    },
    Head {
        key: ItemKey,
        url: String,
    },
    Patch {
        key: ItemKey,
        url: String,
        offset: u64,
        len: u64,
    },
    /// Stops the item's in-flight request.
    Abort {
        key: ItemKey,
    },
    /// Terminates the upload on the server.
    Delete {
        url: String,
    },
    /// `tick` is due at that time.
    WakeAt {
        at_ms: i64,
    },
    Refresh {
        volume: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    FolderCreated {
        key: ItemKey,
    },
    Created {
        key: ItemKey,
        url: String,
    },
    /// `sent` counts every byte of the item the server has, not just this request's.
    Progress {
        key: ItemKey,
        sent: u64,
    },
    Patched {
        key: ItemKey,
        offset: u64,
    },
    HeadDone {
        key: ItemKey,
        offset: u64,
    },
    Failed {
        key: ItemKey,
        failure: ApiFailure,
    },
    Decide {
        key: ItemKey,
        decision: Decision,
        apply_to_all: bool,
    },
    Cancel {
        key: ItemKey,
    },
}
