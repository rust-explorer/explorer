use cap_std::fs::{Dir, Metadata};

use super::read::{Identity, fs_path};
use super::{RelPath, StorageError, Volume};

/// The checks [`Volume::refuse_reserved`] makes by path, made again on the very handle an
/// operation then uses, so that no rename in between can make it reach the staging area.
impl Volume {
    /// Refuses, as not found, acting on `name` in the folder `dir`: `dir` must not be the
    /// staging directory, nor the volume root when `name` is the staging name.
    pub(super) fn refuse_staging_in(
        &self,
        dir: &Dir,
        name: &str,
        path: &RelPath,
    ) -> Result<(), StorageError> {
        let reserved = self.reserved()?;
        let meta = dir
            .dir_metadata()
            .map_err(|err| StorageError::from_io(path, err))?;
        let id = Identity::of(&meta);
        if reserved.hides(id, name, Some(id)) {
            return Err(StorageError::NotFound(path.clone()));
        }
        Ok(())
    }

    /// The parent folder of `path`, opened through symlinks and checked, with the last
    /// component and its metadata; `None` for the root, or when the last component is a
    /// symlink, which the caller resolves from the root. A missing last component is not
    /// found: resolved again from the root, the path could lead elsewhere by then.
    pub(super) fn in_checked_parent<'a>(
        &self,
        path: &'a RelPath,
    ) -> Result<Option<(Dir, &'a str, Metadata)>, StorageError> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(None);
        };
        let dir = self
            .root
            .open_dir(fs_path(&parent))
            .map_err(|err| StorageError::from_io(&parent, err))?;
        self.refuse_staging_in(&dir, name, path)?;
        match dir.symlink_metadata(name) {
            Ok(meta) if meta.is_symlink() => Ok(None),
            Ok(meta) => Ok(Some((dir, name, meta))),
            Err(err) => Err(StorageError::from_io(path, err)),
        }
    }

    /// The parent folder of `path`, opened through symlinks and checked, with the last
    /// component.
    pub(super) fn checked_parent<'a>(
        &self,
        path: &'a RelPath,
    ) -> Result<(Dir, &'a str), StorageError> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(StorageError::IsADirectory(path.clone()));
        };
        let dir = self
            .root
            .open_dir(fs_path(&parent))
            .map_err(|err| StorageError::from_io(&parent, err))?;
        self.refuse_staging_in(&dir, name, path)?;
        Ok((dir, name))
    }
}
