use serde::{Deserialize, Serialize};

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
}
