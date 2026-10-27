use std::io;

use cap_std::fs::Dir;
use rustix::fs::{Mode, OFlags};
use shared::api::conflict::ConflictPolicy;

use crate::storage::read::{Identity, fs_path};
use crate::storage::{EntryKind, InvalidPath, RelPath, StorageError, Volume};

/// More folder levels than any path can have: past this, the walk up has looped.
const MAX_DEPTH: usize = 4096;

/// Whether copying or moving `path` of `source` into the folder `dest_dir` of `dest` would be
/// accepted now. Failures on the source side come as [`StorageError::Source`].
pub fn check_transfer(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
) -> Result<(), StorageError> {
    let Some(name) = path.file_name() else {
        return Err(root_source());
    };
    let kind = source.stat(path).map_err(StorageError::on_source)?.kind;
    if kind == EntryKind::Other {
        return Err(StorageError::NotFound(path.clone()).on_source());
    }
    if dest.stat(dest_dir)?.kind != EntryKind::Dir {
        return Err(StorageError::NotADirectory(dest_dir.clone()));
    }
    if kind == EntryKind::Dir {
        refuse_into_itself(source, path, dest, dest_dir)?;
    }

    let target = dest_dir.join(name)?;
    dest.refuse_reserved(&target)?;
    match dest.root.symlink_metadata(target.as_str()) {
        Ok(_) => match conflict {
            ConflictPolicy::Fail => Err(StorageError::AlreadyExists(target)),
            ConflictPolicy::Replace
                if kind == EntryKind::Dir
                    || dest
                        .root
                        .metadata(target.as_str())
                        .is_ok_and(|meta| meta.is_dir()) =>
            {
                Err(StorageError::IsADirectory(target))
            }
            ConflictPolicy::Replace | ConflictPolicy::KeepBoth => Ok(()),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(StorageError::from_io(&target, err)),
    }
}

fn root_source() -> StorageError {
    InvalidPath {
        reason: "the volume root can't be copied or moved",
    }
    .into()
}

/// Walks up from the real `dest_dir`, whatever symlinks lead to it, to the destination root:
/// meeting the source folder on the way means copying it into itself. A source that is a
/// symlink to its volume root is refused like the root.
fn refuse_into_itself(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
) -> Result<(), StorageError> {
    let source_err = |err| StorageError::from_io(path, err).on_source();
    let folder = Identity::of(&source.root.metadata(path.as_str()).map_err(source_err)?);
    if folder == Identity::of(&source.root.dir_metadata().map_err(source_err)?) {
        return Err(root_source());
    }

    let dest_err = |err| StorageError::from_io(dest_dir, err);
    let root = Identity::of(&dest.root.dir_metadata().map_err(dest_err)?);
    let mut dir = dest.root.open_dir(fs_path(dest_dir)).map_err(dest_err)?;
    for _ in 0..MAX_DEPTH {
        let current = Identity::of(&dir.dir_metadata().map_err(dest_err)?);
        if current == folder {
            return Err(StorageError::InvalidDestination(path.clone()));
        }
        if current == root {
            return Ok(());
        }
        // Read-only, for identities: `..` of a folder inside the volume stays inside it.
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let parent = rustix::fs::openat(&dir, "..", flags, Mode::empty())
            .map_err(|err| dest_err(err.into()))?;
        let parent = Dir::from_std_file(parent.into());
        if Identity::of(&parent.dir_metadata().map_err(dest_err)?) == current {
            return Ok(());
        }
        dir = parent;
    }
    Ok(())
}
