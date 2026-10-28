use serde::{Deserialize, Serialize};

use crate::api::browse::EntryKind;

/// A search stops at this many hits.
pub const SEARCH_LIMIT: usize = 200;
/// Shorter queries, once trimmed, are refused.
pub const MIN_QUERY_CHARS: usize = 2;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Volume-relative path, `/`-separated.
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Milliseconds since the Unix epoch; negative before it.
    pub modified_ms: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SearchResults {
    pub volume: String,
    pub query: String,
    pub hits: Vec<SearchHit>,
    /// The search stopped at its hit limit or, with fewer hits, its time limit.
    pub truncated: bool,
}
