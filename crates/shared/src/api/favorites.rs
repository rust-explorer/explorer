use serde::{Deserialize, Serialize};

/// A bookmarked folder; `exists` is false once the volume or the folder is gone.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    pub volume: String,
    pub path: String,
    pub exists: bool,
}
