use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use shared::api::downloads::ZipSelection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Zips streaming at once, across all users.
pub const ZIP_CONCURRENCY: usize = 4;
/// How long a stored selection can be zipped.
pub const SELECTION_TTL: Duration = Duration::from_secs(10 * 60);
/// Selections a user keeps; a new one beyond this drops the oldest.
pub const MAX_SELECTIONS_PER_USER: usize = 16;

/// What the download routes share: the zip slots and the stored selections.
pub struct Downloads {
    zips: Arc<Semaphore>,
    selections: Mutex<Selections>,
}

#[derive(Default)]
struct Selections {
    by_id: HashMap<String, Stored>,
    next_seq: u64,
}

struct Stored {
    sub: String,
    selection: ZipSelection,
    created: Instant,
    seq: u64,
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new()
    }
}

impl Downloads {
    #[must_use]
    pub fn new() -> Self {
        Self {
            zips: Arc::new(Semaphore::new(ZIP_CONCURRENCY)),
            selections: Mutex::new(Selections::default()),
        }
    }

    /// A zip slot, held until the archive ends; `None` when all are taken.
    #[must_use]
    pub fn try_zip(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.zips).try_acquire_owned().ok()
    }

    /// Stores `selection` for user `sub` and returns its id.
    pub fn remember(&self, sub: &str, selection: ZipSelection) -> String {
        let id = format!("{:032x}", rand::random::<u128>());
        let now = Instant::now();
        let mut selections = self.lock();
        selections
            .by_id
            .retain(|_, stored| now.duration_since(stored.created) < SELECTION_TTL);
        let mut own: Vec<(u64, String)> = selections
            .by_id
            .iter()
            .filter(|(_, stored)| stored.sub == sub)
            .map(|(id, stored)| (stored.seq, id.clone()))
            .collect();
        own.sort_unstable();
        let excess = (own.len() + 1).saturating_sub(MAX_SELECTIONS_PER_USER);
        for (_, old) in own.into_iter().take(excess) {
            selections.by_id.remove(&old);
        }
        let seq = selections.next_seq;
        selections.next_seq += 1;
        selections.by_id.insert(
            id.clone(),
            Stored {
                sub: sub.to_owned(),
                selection,
                created: now,
                seq,
            },
        );
        id
    }

    /// The selection `id` of user `sub`, unless it expired.
    #[must_use]
    pub fn selection(&self, sub: &str, id: &str) -> Option<ZipSelection> {
        self.lock()
            .by_id
            .get(id)
            .filter(|stored| stored.sub == sub && stored.created.elapsed() < SELECTION_TTL)
            .map(|stored| stored.selection.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Selections> {
        self.selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(path: &str) -> ZipSelection {
        ZipSelection {
            volume: "media".to_owned(),
            path: path.to_owned(),
            names: vec!["a".to_owned()],
        }
    }

    #[test]
    fn selections_belong_to_their_user() {
        let downloads = Downloads::new();
        let id = downloads.remember("alice", selection("x"));
        assert_eq!(downloads.selection("alice", &id), Some(selection("x")));
        assert_eq!(downloads.selection("bob", &id), None);
        assert_eq!(downloads.selection("alice", "nope"), None);
    }

    #[test]
    fn a_user_keeps_the_newest_selections() {
        let downloads = Downloads::new();
        let first = downloads.remember("alice", selection("0"));
        let ids: Vec<String> = (1..MAX_SELECTIONS_PER_USER)
            .map(|i| downloads.remember("alice", selection(&i.to_string())))
            .collect();
        let other = downloads.remember("bob", selection("b"));
        assert!(downloads.selection("alice", &first).is_some());
        downloads.remember("alice", selection("new"));
        assert!(downloads.selection("alice", &first).is_none());
        assert!(
            ids.iter()
                .all(|id| downloads.selection("alice", id).is_some())
        );
        assert!(downloads.selection("bob", &other).is_some());
    }

    #[test]
    fn zip_slots_run_out_and_come_back() {
        let downloads = Downloads::new();
        let held: Vec<_> = (0..ZIP_CONCURRENCY)
            .map(|_| downloads.try_zip().unwrap())
            .collect();
        assert!(downloads.try_zip().is_none());
        drop(held);
        assert!(downloads.try_zip().is_some());
    }
}
