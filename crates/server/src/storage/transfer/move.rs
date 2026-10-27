use std::io;

use shared::api::conflict::ConflictPolicy;

use super::{TransferPlan, TransferProgress, check_transfer, copy, plan_transfer};
use crate::storage::read::{Identity, fs_path};
use crate::storage::{InvalidPath, RelPath, StorageError, Volume, WalkItem};

/// Where a moved entry ended up, and how it got there.
#[derive(Debug)]
pub enum MoveOutcome {
    Renamed(RelPath),
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
        return Err(StorageError::InvalidDestination(dest_dir.clone()));
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
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(InvalidPath {
            reason: "the volume root can't be moved",
        }
        .into());
    };
    source
        .refuse_reserved(path)
        .map_err(StorageError::on_source)?;
    if source.name() == dest.name() {
        match rename(source, &parent, name, dest_dir, conflict) {
            Err(StorageError::CrossDevice(_)) => {}
            renamed => return renamed.map(MoveOutcome::Renamed),
        }
    }

    let planned;
    let plan = if let Some(plan) = plan {
        plan
    } else {
        planned = plan_transfer(source, path)?;
        progress.planned(&planned);
        &planned
    };
    let final_path = copy(source, path, dest, dest_dir, conflict, plan, progress)?;
    remove_moved(source, path, plan).map_err(StorageError::on_source)?;
    Ok(MoveOutcome::Copied { final_path })
}

fn rename(
    volume: &Volume,
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
    volume.place(&from, name, &to, dest_dir, name, conflict)
}

/// Removes from the source what the copy took: its files, then its folders once empty. A
/// symlink moved as its target's copy goes alone; its target stays.
fn remove_moved(source: &Volume, path: &RelPath, plan: &TransferPlan) -> Result<(), StorageError> {
    let top = source
        .root
        .symlink_metadata(path.as_str())
        .map_err(|err| StorageError::from_io(path, err))?;
    if !top.is_dir() {
        return remove_file(source, path);
    }
    for item in &plan.items {
        if let WalkItem::File { path, .. } = item {
            remove_file(source, path)?;
        }
    }
    let dirs = plan.items.iter().rev().filter_map(|item| match item {
        WalkItem::Dir(dir) => Some(dir),
        WalkItem::File { .. } => None,
    });
    for dir in dirs.chain([path]) {
        remove_empty_dir(source, dir)?;
    }
    Ok(())
}

/// A file or a symlink, never a folder that took its place since.
fn remove_file(volume: &Volume, path: &RelPath) -> Result<(), StorageError> {
    let (parent, name) = split(path)?;
    volume
        .root
        .open_dir(fs_path(&parent))
        .and_then(|dir| dir.remove_file(name))
        .map_err(|err| StorageError::from_io(path, err))
}

/// A folder that still holds something, as a skipped entry, stays.
fn remove_empty_dir(volume: &Volume, path: &RelPath) -> Result<(), StorageError> {
    let (parent, name) = split(path)?;
    match volume
        .root
        .open_dir(fs_path(&parent))
        .and_then(|dir| dir.remove_dir(name))
    {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(err) => Err(StorageError::from_io(path, err)),
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
