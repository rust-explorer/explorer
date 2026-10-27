use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Dir,
    File,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Milliseconds since the Unix epoch; negative before it.
    pub modified_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Listing {
    pub volume: String,
    pub path: String,
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VolumeInfo {
    pub name: String,
    pub read_only: bool,
    pub space: Option<Space>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CreateFolder {
    pub path: String,
    /// Fail when the name is taken or the parent is missing, instead of merging.
    #[serde(default)]
    pub exclusive: bool,
}
