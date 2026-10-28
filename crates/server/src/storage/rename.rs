use std::io;

use shared::api::conflict::ConflictPolicy;

use super::read::fs_path;
use super::{InvalidPath, RelPath, StorageError, Volume};

impl Volume {
    /// Renames the last component of `path` in place; never follows a final symlink.
    pub fn rename(
        &self,
        path: &RelPath,
        new_name: &str,
        conflict: ConflictPolicy,
    ) -> Result<RelPath, StorageError> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(InvalidPath {
                reason: "the volume root can't be renamed",
            }
            .into());
        };
        let target = parent.join(new_name)?;
        self.refuse_reserved(path)?;
        self.refuse_reserved(&target)?;
        let dir = self
            .root
            .open_dir(fs_path(&parent))
            .map_err(|err| StorageError::from_io(&parent, err))?;
        dir.symlink_metadata(name)
            .map_err(|err| StorageError::from_io(path, err))?;
        if new_name == name {
            return Ok(path.clone());
        }
        if conflict == ConflictPolicy::Fail {
            match dir.symlink_metadata(new_name) {
                Ok(_) => return Err(StorageError::AlreadyExists(target)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(StorageError::from_io(&target, err)),
            }
        }
        self.place(&dir, name, &dir, &parent, new_name, conflict)
    }
}
