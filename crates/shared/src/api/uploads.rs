use serde::{Deserialize, Serialize};

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
