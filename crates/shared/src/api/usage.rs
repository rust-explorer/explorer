use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
    Dir,
    File,
    /// The files too small to be listed one by one, together.
    OtherFiles,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageItem {
    /// Empty for `OtherFiles`.
    pub name: String,
    pub kind: UsageKind,
    /// Allocated on disk.
    pub bytes: u64,
    pub files: u64,
    /// Something under this folder could not be read.
    pub incomplete: bool,
}

/// One folder of a finished usage scan.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UsageView {
    pub volume: String,
    /// The folder the scan started from.
    pub root: String,
    /// The folder shown: `root` or below it.
    pub path: String,
    /// Milliseconds since the Unix epoch.
    pub scanned_at_ms: i64,
    pub bytes: u64,
    pub files: u64,
    /// Sub-folders at any depth.
    pub dirs: u64,
    /// Something under this folder could not be read.
    pub incomplete: bool,
    /// The scan's folder limit was reached: its sub-folders are counted here, not listed.
    pub collapsed: bool,
    /// Biggest first.
    pub items: Vec<UsageItem>,
}
