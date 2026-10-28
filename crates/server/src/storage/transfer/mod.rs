mod check;
mod copy;
mod mode;
mod r#move;
mod remove;

pub use check::check_transfer;
pub use copy::copy;
pub use r#move::{MoveOutcome, check_move, move_entry};

use std::cell::Cell;

use super::{EntryKind, EntryMeta, RelPath, StorageError, Volume, WalkItem};

/// The most entries a copy or a cross-volume move takes on, so that its plan stays a few
/// hundred megabytes at most.
pub const MAX_TRANSFER_ENTRIES: usize = 1_000_000;

/// How much of a file is read and written at a time; cancellation is checked in between.
pub const COPY_CHUNK: usize = 1024 * 1024;

/// What a copy or move reports as it goes.
pub trait TransferProgress {
    fn is_cancelled(&self) -> bool;
    fn add_bytes(&self, n: u64);
    fn add_file(&self);
    fn skip(&self);
    /// A move that found it has to copy made this plan itself.
    fn planned(&self, _plan: &TransferPlan) {}
}

/// What copying an entry involves: a file is one item, a folder its walk, without the folder
/// itself. `skipped` counts the entries the walk leaves out.
#[derive(Debug)]
pub struct TransferPlan {
    pub items: Vec<WalkItem>,
    pub files: u64,
    pub bytes: u64,
    pub skipped: u64,
}

/// Plans copying `path` of `source`, failing with `Cancelled` as soon as `progress` is cancelled
/// and with `TooManyEntries` past [`MAX_TRANSFER_ENTRIES`].
pub fn plan_transfer(
    source: &Volume,
    path: &RelPath,
    progress: &dyn TransferProgress,
) -> Result<TransferPlan, StorageError> {
    plan(source, path, progress, MAX_TRANSFER_ENTRIES).map_err(StorageError::on_source)
}

fn plan(
    source: &Volume,
    path: &RelPath,
    progress: &dyn TransferProgress,
    max_entries: usize,
) -> Result<TransferPlan, StorageError> {
    let meta = source.stat(path)?;
    let (items, skipped) = match meta.kind {
        EntryKind::File => (
            vec![WalkItem::File {
                path: path.clone(),
                size: meta.size.unwrap_or(0),
                modified: meta.modified,
            }],
            0,
        ),
        EntryKind::Dir => walk(source, path, progress, max_entries)?,
        EntryKind::Other => return Err(StorageError::NotFound(path.clone())),
    };
    let (mut files, mut bytes) = (0, 0);
    for item in &items {
        if let WalkItem::File { size, .. } = item {
            files += 1;
            bytes += size;
        }
    }
    Ok(TransferPlan {
        items,
        files,
        bytes,
        skipped,
    })
}

/// Like [`Volume::walk_counting`], listing at most `max_entries` entries in all.
fn walk(
    source: &Volume,
    dir: &RelPath,
    progress: &dyn TransferProgress,
    max_entries: usize,
) -> Result<(Vec<WalkItem>, u64), StorageError> {
    let listed = Cell::new(0);
    let go_on = || {
        listed.set(listed.get() + 1);
        !progress.is_cancelled() && listed.get() <= max_entries
    };
    let mut pending = Vec::new();
    let mut skipped = 0;
    let mut list = |path: &RelPath, pending: &mut Vec<(RelPath, EntryMeta)>| {
        let entries = match source.entries_while(path, &go_on) {
            Err(StorageError::Cancelled) if !progress.is_cancelled() => {
                return Err(StorageError::TooManyEntries(dir.clone()));
            }
            entries => entries?,
        };
        for entry in entries.into_iter().rev() {
            match entry.path {
                Some(child)
                    if entry.meta.kind == EntryKind::File
                        || (entry.meta.kind == EntryKind::Dir && source.is_real_dir(&child)) =>
                {
                    pending.push((child, entry.meta));
                }
                _ => skipped += 1,
            }
        }
        Ok(())
    };
    list(dir, &mut pending)?;
    let mut items = Vec::new();
    while let Some((path, meta)) = pending.pop() {
        if meta.kind == EntryKind::File {
            items.push(WalkItem::File {
                path,
                size: meta.size.unwrap_or(0),
                modified: meta.modified,
            });
        } else {
            list(&path, &mut pending)?;
            items.push(WalkItem::Dir(path));
        }
    }
    Ok((items, skipped))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::storage::Volumes;

    /// Answers `cancelled` once asked more than `after` times.
    struct CancelAfter {
        asked: Cell<usize>,
        after: usize,
    }

    impl TransferProgress for CancelAfter {
        fn is_cancelled(&self) -> bool {
            self.asked.set(self.asked.get() + 1);
            self.asked.get() > self.after
        }
        fn add_bytes(&self, _n: u64) {}
        fn add_file(&self) {}
        fn skip(&self) {}
    }

    fn going_on() -> CancelAfter {
        CancelAfter {
            asked: Cell::new(0),
            after: usize::MAX,
        }
    }

    #[test]
    fn planning_stops_past_the_limit_or_when_cancelled() {
        let data = tempfile::tempdir().unwrap();
        for dir in ["F/a", "F/b"] {
            fs::create_dir_all(data.path().join("media").join(dir)).unwrap();
            for file in ["1", "2", "3"] {
                fs::write(data.path().join("media").join(dir).join(file), "x").unwrap();
            }
        }
        let volumes = Volumes::discover(data.path()).unwrap();
        let media = volumes.get("media").unwrap();
        let folder = RelPath::parse("F").unwrap();

        let all = plan(media, &folder, &going_on(), 8).unwrap();
        assert_eq!((all.items.len(), all.files, all.skipped), (8, 6, 0));
        assert!(matches!(
            plan(media, &folder, &going_on(), 7),
            Err(StorageError::TooManyEntries(path)) if path == folder
        ));
        let cancel = CancelAfter {
            asked: Cell::new(0),
            after: 3,
        };
        assert!(matches!(
            plan(media, &folder, &cancel, 8),
            Err(StorageError::Cancelled)
        ));
        assert!(cancel.asked.get() <= 5, "{}", cancel.asked.get());
    }
}
