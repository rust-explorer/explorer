use rustix::io::Errno;

use shared::api::conflict::ConflictPolicy;

use super::copy::{Taken, copy_taking};
use super::remove::{Expect, Removed, remove_if};
use super::{TransferPlan, TransferProgress, check_transfer, plan_transfer};
use crate::storage::read::{Identity, fs_path};
use crate::storage::{InvalidPath, RelPath, StorageError, Volume, WalkItem};

/// Where a moved entry ended up, and how it got there.
#[derive(Debug)]
pub enum MoveOutcome {
    Renamed { final_path: RelPath },
    Copied { final_path: RelPath },
}

/// Like [`check_transfer`], and moving an entry into the folder it is already in is refused.
pub fn check_move(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
) -> Result<(), StorageError> {
    if is_own_folder(source, path, dest, dest_dir) {
        return Err(StorageError::InvalidDestination(path.clone()));
    }
    check_transfer(source, path, dest, dest_dir, conflict)
}

/// Errors are left to [`check_transfer`], which reports them.
fn is_own_folder(source: &Volume, path: &RelPath, dest: &Volume, dest_dir: &RelPath) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    match (
        source.root.metadata(fs_path(&parent)),
        dest.root.metadata(fs_path(dest_dir)),
    ) {
        (Ok(parent), Ok(dir)) => Identity::of(&parent) == Identity::of(&dir),
        _ => false,
    }
}

/// Same volume: one no-replace rename via `place` (the entry itself moves, symlinks included).
/// Other volume, or `CrossDevice` from the rename: `copy`, then delete each copied file from the
/// source and remove the source folders that ended up empty, deepest first.
///
/// A failed or cancelled copy leaves the source whole and nothing in the destination; once the
/// copy is complete, the deletes run to the end, and a failure there leaves both copies.
pub fn move_entry(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
    plan: Option<&TransferPlan>,
    progress: &dyn TransferProgress,
) -> Result<MoveOutcome, StorageError> {
    let (parent, name) = split(path)?;
    source
        .refuse_reserved(path)
        .map_err(StorageError::on_source)?;
    if source.name() == dest.name() {
        match rename(source, path, &parent, name, dest_dir, conflict) {
            Err(StorageError::CrossDevice(_)) => {}
            renamed => return renamed.map(|final_path| MoveOutcome::Renamed { final_path }),
        }
    }

    let planned;
    let plan = if let Some(plan) = plan {
        plan
    } else {
        planned = plan_transfer(source, path, progress)?;
        progress.planned(&planned);
        &planned
    };
    let (final_path, taken) = copy_taking(source, path, dest, dest_dir, conflict, plan, progress)?;
    remove_moved(source, path, plan, &taken).map_err(StorageError::on_source)?;
    Ok(MoveOutcome::Copied { final_path })
}

fn rename(
    volume: &Volume,
    path: &RelPath,
    parent: &RelPath,
    name: &str,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
) -> Result<RelPath, StorageError> {
    volume.refuse_reserved(&dest_dir.join(name)?)?;
    let from = volume
        .root
        .open_dir(fs_path(parent))
        .map_err(|err| StorageError::from_io(parent, err).on_source())?;
    let to = volume
        .root
        .open_dir(fs_path(dest_dir))
        .map_err(|err| StorageError::from_io(dest_dir, err))?;
    volume
        .refuse_staging_in(&from, name, path)
        .map_err(StorageError::on_source)?;
    volume.refuse_staging_in(&to, name, &dest_dir.join(name)?)?;
    // `EINVAL`: a folder renamed into itself.
    volume
        .place(&from, name, &to, dest_dir, name, conflict)
        .map_err(|err| match err {
            StorageError::Io { source, .. }
                if source.raw_os_error() == Some(Errno::INVAL.raw_os_error()) =>
            {
                StorageError::InvalidDestination(path.clone())
            }
            other => other,
        })
}

/// Removes from the source what the copy took: its files, then its folders once empty, each
/// only if it is still what was copied. A symlink moved as its target's copy goes alone, if it
/// still leads there; its target stays. A file changed since it was copied stays, and so do the
/// folders holding it: the move then fails, both copies kept.
fn remove_moved(
    source: &Volume,
    path: &RelPath,
    plan: &TransferPlan,
    taken: &Taken,
) -> Result<(), StorageError> {
    let mut changed = None;
    let mut remove = |at: &RelPath, expect: Expect| -> Result<(), StorageError> {
        if remove_if(source, at, expect, true)? == Removed::Changed && changed.is_none() {
            changed = Some(at.clone());
        }
        Ok(())
    };
    if matches!(taken.top, Expect::Dir(_)) && source.is_real_dir(path) {
        for (index, stamp) in &taken.files {
            remove(item_path(plan, *index), Expect::File(*stamp))?;
        }
        for (index, id) in taken.dirs.iter().rev() {
            remove(item_path(plan, *index), Expect::Dir(*id))?;
        }
    }
    remove(path, taken.top)?;
    match changed {
        Some(path) => Err(StorageError::ChangedDuringMove(path)),
        None => Ok(()),
    }
}

fn item_path(plan: &TransferPlan, index: usize) -> &RelPath {
    match &plan.items[index] {
        WalkItem::Dir(path) | WalkItem::File { path, .. } => path,
    }
}

fn split(path: &RelPath) -> Result<(RelPath, &str), StorageError> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok((parent, name)),
        _ => Err(InvalidPath {
            reason: "the volume root can't be moved",
        }
        .into()),
    }
}
