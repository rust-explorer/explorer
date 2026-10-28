use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

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
    /// Case- and accent-insensitive substring match on names, whatever their Unicode
    /// normalisation form, breadth-first from the root, so shallow matches
    /// come first. Follows `walk`'s rules for the staging folder and symlinks. Stops early, with
    /// `truncated`, at `limit` hits or at `deadline`, and with `Cancelled` once `cancel` is set.
    pub fn search(
        &self,
        needle: &str,
        limit: usize,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<SearchOutcome, StorageError> {
        let needle = folded(needle);
        let mut hits = Vec::new();
        let mut pending = VecDeque::from([RelPath::root()]);
        let go_on = || !cancel.load(Ordering::Relaxed) && Instant::now() < deadline;
        while let Some(dir) = pending.pop_front() {
            let listed = if go_on() {
                self.entries_while(&dir, &go_on)
            } else {
                Err(StorageError::Cancelled)
            };
            let entries = match listed {
                Ok(entries) => entries,
                Err(StorageError::Cancelled) if cancel.load(Ordering::Relaxed) => {
                    return Err(StorageError::Cancelled);
                }
                Err(StorageError::Cancelled) => {
                    return Ok(SearchOutcome {
                        hits,
                        truncated: true,
                    });
                }
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
                if folded(&entry.meta.name).contains(&needle) {
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

/// Decomposed, without combining marks, lowercased: `Été` (NFC or NFD) and `ete` fold alike.
fn folded(text: &str) -> String {
    text.nfd()
        .filter(|&c| !is_combining_mark(c))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::Duration;

    use super::*;
    use crate::storage::Volumes;

    #[test]
    fn listing_stops_inside_a_folder() {
        let data = tempfile::tempdir().unwrap();
        let media = data.path().join("media");
        std::fs::create_dir(&media).unwrap();
        for i in 0..50 {
            std::fs::write(media.join(format!("f{i}")), "").unwrap();
        }
        let volumes = Volumes::discover(data.path()).unwrap();
        let volume = volumes.get("media").unwrap();

        let asked = Cell::new(0);
        let result = volume
            .entries_while(&RelPath::root(), &|| {
                asked.set(asked.get() + 1);
                asked.get() <= 10
            })
            .map(drop);
        assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
        assert_eq!(asked.get(), 11);

        let cancel = AtomicBool::new(true);
        let later = Instant::now() + Duration::from_secs(60);
        let result = volume.search("f", 10, later, &cancel);
        assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
        let past = Instant::now();
        let outcome = volume
            .search("f", 10, past, &AtomicBool::new(false))
            .unwrap();
        assert!(outcome.truncated && outcome.hits.is_empty());
    }

    #[test]
    fn matches_across_normalisation_forms_and_accents() {
        let data = tempfile::tempdir().unwrap();
        let media = data.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(media.join("E\u{301}te\u{301}.txt"), "").unwrap();
        std::fs::write(media.join("\u{c9}cole.txt"), "").unwrap();
        let volumes = Volumes::discover(data.path()).unwrap();
        let volume = volumes.get("media").unwrap();
        let later = Instant::now() + Duration::from_secs(60);
        let hits = |needle: &str| {
            volume
                .search(needle, 10, later, &AtomicBool::new(false))
                .unwrap()
                .hits
                .len()
        };
        assert_eq!(hits("\u{e9}t\u{e9}"), 1);
        assert_eq!(hits("ETE"), 1);
        assert_eq!(hits("e\u{301}cole"), 1);
        assert_eq!(hits("ecole"), 1);
        assert_eq!(hits("xyz"), 0);
    }
}
