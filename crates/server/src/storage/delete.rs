use cap_std::fs::{Dir, Metadata};

use super::read::fs_path;
use super::{InvalidPath, RelPath, StorageError, Volume};

impl Volume {
    /// Removes a file, a symlink (never its target) or a whole folder tree.
    pub fn delete(&self, path: &RelPath) -> Result<(), StorageError> {
        let (dir, name, meta) = self.locate(path)?;
        let removed = if meta.is_dir() {
            dir.remove_dir_all(name)
        } else {
            dir.remove_file(name)
        };
        removed.map_err(|err| StorageError::from_io(path, err))
    }

    /// Fails as [`Volume::delete`] would before removing anything: the root, a reserved path or
    /// a missing entry.
    pub fn check_deletable(&self, path: &RelPath) -> Result<(), StorageError> {
        self.locate(path).map(drop)
    }

    /// The parent directory, opened through symlinks, and the last component, not followed.
    fn locate<'a>(&self, path: &'a RelPath) -> Result<(Dir, &'a str, Metadata), StorageError> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(InvalidPath {
                reason: "the volume root can't be removed",
            }
            .into());
        };
        self.refuse_reserved(path)?;
        let dir = self
            .root
            .open_dir(fs_path(&parent))
            .map_err(|err| StorageError::from_io(&parent, err))?;
        let meta = dir
            .symlink_metadata(name)
            .map_err(|err| StorageError::from_io(path, err))?;
        Ok((dir, name, meta))
    }
}
