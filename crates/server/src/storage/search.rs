use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use super::read::EntryKind;
use super::{RelPath, StorageError, Volume};

pub use shared::api::search::SEARCH_LIMIT;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: RelPath,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    pub truncated: bool,
}

impl Volume {
    /// Case-insensitive substring match on names, breadth-first from the root, so shallow matches
    /// come first. Follows `walk`'s rules for the staging folder and symlinks. Stops early, with
    /// `truncated`, at `limit` hits or at `deadline`, and with `Cancelled` once `cancel` is set.
    pub fn search(
        &self,
        needle: &str,
        limit: usize,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<SearchOutcome, StorageError> {
        let needle = needle.to_lowercase();
        let mut hits = Vec::new();
        let mut pending = VecDeque::from([RelPath::root()]);
        while let Some(dir) = pending.pop_front() {
            if cancel.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Ok(SearchOutcome {
                    hits,
                    truncated: true,
                });
            }
            let entries = match self.entries(&dir) {
                Ok(entries) => entries,
                Err(err) if dir.is_root() => return Err(err),
                Err(err) => {
                    tracing::debug!(volume = self.name(), dir = dir.as_str(), error = %err, "search skips an unreadable folder");
                    continue;
                }
            };
            for entry in entries {
                let Some(path) = entry.path else { continue };
                let is_dir = entry.meta.kind == EntryKind::Dir;
                if entry.meta.kind == EntryKind::Other || (is_dir && !self.is_real_dir(&path)) {
                    continue;
                }
                if entry.meta.name.to_lowercase().contains(&needle) {
                    if hits.len() >= limit {
                        return Ok(SearchOutcome {
                            hits,
                            truncated: true,
                        });
                    }
                    hits.push(SearchHit {
                        path: path.clone(),
                        kind: entry.meta.kind,
                        size: entry.meta.size,
                        modified: entry.meta.modified,
                    });
                }
                if is_dir {
                    pending.push_back(path);
                }
            }
        }
        Ok(SearchOutcome {
            hits,
            truncated: false,
        })
    }
}
