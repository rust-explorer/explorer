use std::io;

use cap_std::fs::Dir;
use rustix::fs::{AtFlags, RenameFlags};
use rustix::io::Errno;
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
        self.refuse_staging_in(&dir, name, path)?;
        self.refuse_staging_in(&dir, new_name, &target)?;
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

/// A rename that fails with `EEXIST` rather than replace anything. Where the filesystem has no
/// such rename, a file is hard-linked then unlinked, which is just as exclusive; a folder, which
/// can't be linked, is renamed only if nothing is there, so at worst it replaces an empty folder
/// created in between.
pub(super) fn rename_noreplace(
    from_dir: &Dir,
    from_name: &str,
    to_dir: &Dir,
    to_name: &str,
) -> Result<(), Errno> {
    match rustix::fs::renameat_with(from_dir, from_name, to_dir, to_name, RenameFlags::NOREPLACE) {
        Err(err) if is_unsupported(err) => {
            rename_without_noreplace(from_dir, from_name, to_dir, to_name)
        }
        result => result,
    }
}

/// `EINVAL` is also a folder moved into itself, which the plain rename reports again.
fn is_unsupported(err: Errno) -> bool {
    [Errno::INVAL, Errno::NOSYS, Errno::NOTSUP, Errno::OPNOTSUPP].contains(&err)
}

fn rename_without_noreplace(
    from_dir: &Dir,
    from_name: &str,
    to_dir: &Dir,
    to_name: &str,
) -> Result<(), Errno> {
    let errno = |err: io::Error| Errno::from_io_error(&err).unwrap_or(Errno::IO);
    let source = from_dir.symlink_metadata(from_name).map_err(errno)?;
    if source.is_dir() {
        return match to_dir.symlink_metadata(to_name) {
            Ok(_) => Err(Errno::EXIST),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                rustix::fs::renameat(from_dir, from_name, to_dir, to_name)
            }
            Err(err) => Err(errno(err)),
        };
    }
    rustix::fs::linkat(from_dir, from_name, to_dir, to_name, AtFlags::empty())?;
    if let Err(err) = rustix::fs::unlinkat(from_dir, from_name, AtFlags::empty()) {
        let _ = rustix::fs::unlinkat(to_dir, to_name, AtFlags::empty());
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    use cap_std::ambient_authority;

    use super::*;

    #[test]
    fn rename_without_noreplace_never_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        fs::write(path.join("file"), "new").unwrap();
        fs::write(path.join("taken"), "old").unwrap();
        fs::create_dir(path.join("dir")).unwrap();
        fs::write(path.join("dir/inner"), "x").unwrap();
        fs::create_dir(path.join("empty")).unwrap();
        let dir = Dir::open_ambient_dir(path, ambient_authority()).unwrap();

        for (from, to) in [
            ("file", "taken"),
            ("file", "empty"),
            ("dir", "empty"),
            ("dir", "taken"),
        ] {
            assert_eq!(
                rename_without_noreplace(&dir, from, &dir, to),
                Err(Errno::EXIST),
                "{from} -> {to}"
            );
        }
        assert_eq!(fs::read_to_string(path.join("taken")).unwrap(), "old");
        assert!(path.join("empty").is_dir());

        rename_without_noreplace(&dir, "file", &dir, "moved").unwrap();
        rename_without_noreplace(&dir, "dir", &dir, "moved-dir").unwrap();
        assert!(!path.join("file").exists());
        assert_eq!(fs::read_to_string(path.join("moved")).unwrap(), "new");
        assert_eq!(fs::metadata(path.join("moved")).unwrap().nlink(), 1);
        assert_eq!(
            fs::read_to_string(path.join("moved-dir/inner")).unwrap(),
            "x"
        );
        assert_eq!(
            rename_without_noreplace(&dir, "moved-dir", &dir, "moved-dir/sub"),
            Err(Errno::INVAL)
        );
    }
}
