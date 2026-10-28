use std::fs::{File, Permissions};
use std::os::unix::fs::PermissionsExt as _;

use rustix::fs::{Mode, OFlags};

use crate::storage::read::{Identity, fs_path};
use crate::storage::{RelPath, Volume};

/// The permission bits a copy keeps: read, write and execute for each class. Set-id and sticky
/// bits are dropped, as the container user could not keep them on a file it creates anyway.
pub(super) fn kept_mode(mode: u32) -> u32 {
    mode & 0o777
}

/// Gives a copied file its source's permissions. A file the owner can't write gets them only
/// once placed (`placed`), since placing it opens it for writing; until then it has them plus
/// owner write, never more than its source otherwise.
pub(super) fn set_file_mode(file: &File, mode: u32, placed: bool, volume: &Volume) {
    let mode = if placed { mode } else { mode | 0o200 };
    if placed && mode & 0o200 != 0 {
        return;
    }
    if let Err(err) = file.set_permissions(Permissions::from_mode(mode)) {
        tracing::debug!(volume = volume.name(), error = %err, "cannot set a copied file's permissions");
    }
}

/// Gives the copied folder at `path` the permissions `mode`, if it is still the folder
/// `created`.
/// Done once its content is in, since a folder may not let its owner write in it.
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
