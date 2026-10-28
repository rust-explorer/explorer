mod check;
mod copy;
mod r#move;

pub use check::check_transfer;
pub use copy::copy;
pub use r#move::{MoveOutcome, check_move, move_entry};

use super::{EntryKind, RelPath, StorageError, Volume, WalkItem};

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

pub fn plan_transfer(source: &Volume, path: &RelPath) -> Result<TransferPlan, StorageError> {
    plan(source, path, &|| true).map_err(StorageError::on_source)
}

/// Like [`plan_transfer`], failing with `Cancelled` as soon as `progress` is cancelled, even
/// in the middle of a large walk.
pub fn plan_transfer_with(
    source: &Volume,
    path: &RelPath,
    progress: &dyn TransferProgress,
) -> Result<TransferPlan, StorageError> {
    plan(source, path, &|| !progress.is_cancelled()).map_err(StorageError::on_source)
}

fn plan(
    source: &Volume,
    path: &RelPath,
    go_on: &dyn Fn() -> bool,
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
        EntryKind::Dir => source.walk_counting_while(path, go_on)?,
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
