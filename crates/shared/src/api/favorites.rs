use serde::{Deserialize, Serialize};

/// Favorites one user can have.
pub const MAX_FAVORITES: usize = 100;

/// A bookmarked folder; `exists` is false once the volume or the folder is gone.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    pub volume: String,
    pub path: String,
    pub exists: bool,
}
