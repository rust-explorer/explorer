use serde::{Deserialize, Serialize};

use super::conflict::ConflictPolicy;

/// Unfinished uploads a user may have at once; more are refused with `too_many_uploads`.
pub const MAX_PENDING_UPLOADS: usize = 1000;
/// `PATCH` requests a user may have in flight at once; more are refused with
/// `too_many_patches`, which clients retry later. The web app sends at most 3 per tab.
pub const MAX_PATCHES_PER_USER: usize = 8;

/// An upload of the current user that is not finished yet.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PendingUpload {
    pub id: String,
    pub volume: String,
    pub path: String,
    pub length: u64,
    pub offset: u64,
    pub modified_ms: Option<i64>,
    /// What to do when the destination exists, chosen when the upload was created.
    #[serde(default)]
    pub conflict: ConflictPolicy,
    /// The client's [`crate::tus::fingerprint`] of the file, when it sent one.
    #[serde(default)]
    pub fingerprint: Option<String>,
}
