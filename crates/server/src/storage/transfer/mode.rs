use std::fs::{File, Permissions};
use std::os::unix::fs::PermissionsExt as _;

use rustix::fs::{Mode, OFlags};

use crate::storage::read::{Identity, fs_path};
use crate::storage::{RelPath, Volume};

/// The permission bits a copied file keeps: read, write and execute for each class, plus owner
/// read and write, since the copy belongs to the container user, who must still be able to
/// read, replace and delete it. Set-id and sticky bits are dropped, as the container user could
/// not keep them on a file it creates anyway.
pub(super) fn kept_file_mode(mode: u32) -> u32 {
    mode & 0o777 | 0o600
}

/// Like [`kept_file_mode`], for a folder, which keeps owner read, write and search.
pub(super) fn kept_dir_mode(mode: u32) -> u32 {
    mode & 0o777 | 0o700
}

/// Gives a copied file its source's permissions, as [`kept_file_mode`] made them.
pub(super) fn set_file_mode(file: &File, mode: u32, volume: &Volume) {
    if let Err(err) = file.set_permissions(Permissions::from_mode(mode)) {
        tracing::debug!(volume = volume.name(), error = %err, "cannot set a copied file's permissions");
    }
}

/// Gives the copied folder at `path` the permissions `mode`, if it is still the folder
/// `created`, once its content is in.
pub(super) fn set_dir_mode(volume: &Volume, path: &RelPath, created: Identity, mode: u32) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let set = || -> std::io::Result<bool> {
        let parent = volume.root.open_dir(fs_path(&parent))?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let dir = File::from(rustix::fs::openat(&parent, name, flags, Mode::empty())?);
        let meta = cap_std::fs::Metadata::from_file(&dir)?;
        if Identity::of(&meta) != created {
            return Ok(false);
        }
        dir.set_permissions(Permissions::from_mode(mode))?;
        Ok(true)
    };
    match set() {
        Ok(true) => {}
        Ok(false) => tracing::debug!(
            volume = volume.name(),
            path = path.as_str(),
            "a copied folder was replaced; its permissions are left alone"
        ),
        Err(err) => tracing::debug!(
            volume = volume.name(),
            path = path.as_str(),
            error = %err,
            "cannot set a copied folder's permissions"
        ),
    }
}
