use std::io;
use std::time::SystemTime;

use cap_std::fs::Dir;
use rustix::fs::{Mode, OFlags, RenameFlags};
use rustix::io::Errno;
use shared::api::uploads::ConflictPolicy;

use super::path::STAGING_DIR;
use super::read::fs_path;
use super::{RelPath, Staging, StorageError, UploadId, Volume};

const KEEP_BOTH_MAX: u32 = 999;

/// `Film.mkv` → `Film (n).mkv`. A leading or trailing dot does not start an extension.
#[must_use]
pub fn keep_both_name(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => {
            format!("{} ({n}){}", &name[..dot], &name[dot..])
        }
        _ => format!("{name} ({n})"),
    }
}

/// `a/b/c` → `a`, `a/b`, `a/b/c`.
pub(super) fn prefixes(path: &RelPath) -> Vec<RelPath> {
    let mut prefixes: Vec<RelPath> = Vec::new();
    for component in path.as_str().split('/').filter(|c| !c.is_empty()) {
        let parent = prefixes.last().cloned().unwrap_or_else(RelPath::root);
        let next = parent
            .join(component)
            .expect("a component of a valid path is a valid name");
        prefixes.push(next);
    }
    prefixes
}

/// Like the read operations, paths are resolved from the volume root with symlinks followed and
/// confined by cap-std; renames then only take a `Dir` handle and a single name on each side.
impl Volume {
    pub fn create_dir_all(&self, dir: &RelPath) -> Result<(), StorageError> {
        self.refuse_reserved(dir)?;
        for prefix in prefixes(dir) {
            match self.is_dir_at(&prefix)? {
                Some(true) => {}
                Some(false) if prefix == *dir => return Err(StorageError::AlreadyExists(prefix)),
                Some(false) => return Err(StorageError::NotADirectory(prefix)),
                None => match self.root.create_dir(prefix.as_str()) {
                    Ok(()) => {}
                    Err(err)
                        if err.kind() == io::ErrorKind::AlreadyExists
                            && self.is_dir_at(&prefix)? == Some(true) => {}
                    Err(err) => return Err(StorageError::from_io(&prefix, err)),
                },
            }
        }
        Ok(())
    }

    /// Whether an upload to `path` would be accepted now; missing parent folders are fine.
    pub fn check_upload_target(
        &self,
        path: &RelPath,
        conflict: ConflictPolicy,
    ) -> Result<(), StorageError> {
        let Some(parent) = path.parent() else {
            return Err(StorageError::IsADirectory(path.clone()));
        };
        self.refuse_reserved(path)?;
        for prefix in prefixes(&parent) {
            match self.is_dir_at(&prefix)? {
                Some(true) => {}
                Some(false) => return Err(StorageError::NotADirectory(prefix)),
                None => return Ok(()),
            }
        }
        match conflict {
            ConflictPolicy::Fail => match self.root.symlink_metadata(path.as_str()) {
                Ok(_) => Err(StorageError::AlreadyExists(path.clone())),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(StorageError::from_io(path, err)),
            },
            ConflictPolicy::Replace if self.is_dir_at(path)? == Some(true) => {
                Err(StorageError::IsADirectory(path.clone()))
            }
            ConflictPolicy::Replace | ConflictPolicy::KeepBoth => Ok(()),
        }
    }

    pub fn staging(&self) -> Result<Staging, StorageError> {
        if let Some(staging) = self.existing_staging()? {
            return Ok(staging);
        }
        match rustix::fs::mkdirat(&self.root, STAGING_DIR, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(err) => return Err(staging_error(err)),
        }
        self.existing_staging()?
            .ok_or_else(|| staging_error(Errno::NOENT))
    }

    /// The staging directory is opened without following symlinks: anything but a real directory
    /// under that name is an error.
    pub fn existing_staging(&self) -> Result<Option<Staging>, StorageError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        match rustix::fs::openat(&self.root, STAGING_DIR, flags, Mode::empty()) {
            Ok(fd) => Ok(Some(Staging {
                dir: Dir::from_std_file(fd.into()),
            })),
            Err(Errno::NOENT) => Ok(None),
            Err(err) => Err(staging_error(err)),
        }
    }

    /// Moves a staged upload to `dest`, creating its parent folders, and returns the path it
    /// ended up at, which differs from `dest` when another file took that name.
    pub fn commit_upload(
        &self,
        staging: &Staging,
        id: &UploadId,
        dest: &RelPath,
        conflict: ConflictPolicy,
        modified: Option<SystemTime>,
    ) -> Result<RelPath, StorageError> {
        let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
            return Err(StorageError::IsADirectory(dest.clone()));
        };
        self.refuse_reserved(dest)?;
        self.create_dir_all(&parent)?;
        let parent_dir = self
            .root
            .open_dir(fs_path(&parent))
            .map_err(|err| StorageError::from_io(&parent, err))?;

        let file = staging.open_write(id)?;
        let staged_err = |err| StorageError::from_io(&RelPath::root(), err);
        if let Some(modified) = modified {
            file.set_modified(modified).map_err(staged_err)?;
        }
        file.sync_all().map_err(staged_err)?;
        drop(file);

        if conflict == ConflictPolicy::Replace {
            if self.is_dir_at(dest)? == Some(true) {
                return Err(StorageError::IsADirectory(dest.clone()));
            }
            rustix::fs::renameat(&staging.dir, id.as_str(), &parent_dir, name)
                .map_err(|err| rename_error(dest, err))?;
            return Ok(dest.clone());
        }

        for n in 0..=KEEP_BOTH_MAX {
            let candidate = if n == 0 {
                name.to_owned()
            } else {
                keep_both_name(name, n)
            };
            let Ok(target) = parent.join(&candidate) else {
                break;
            };
            match rustix::fs::renameat_with(
                &staging.dir,
                id.as_str(),
                &parent_dir,
                candidate.as_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    if n > 0 && conflict == ConflictPolicy::Fail {
                        tracing::info!(
                            volume = self.name(),
                            path = dest.as_str(),
                            final_path = target.as_str(),
                            "destination appeared during the upload; keeping both"
                        );
                    }
                    return Ok(target);
                }
                Err(Errno::EXIST) => {}
                Err(err) => return Err(rename_error(&target, err)),
            }
        }
        Err(StorageError::AlreadyExists(dest.clone()))
    }

    /// `None` when nothing (or a dangling symlink) is there.
    fn is_dir_at(&self, path: &RelPath) -> Result<Option<bool>, StorageError> {
        match self.root.metadata(fs_path(path)) {
            Ok(meta) => Ok(Some(meta.is_dir())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(StorageError::from_io(path, err)),
        }
    }
}

/// Both sides are plain names under open `Dir` handles, so `EXDEV` here is a mount point nested
/// in the volume, not an escape.
fn rename_error(path: &RelPath, err: Errno) -> StorageError {
    if err == Errno::XDEV {
        return StorageError::CrossDevice(path.clone());
    }
    StorageError::from_io(path, err.into())
}

fn staging_error(err: Errno) -> StorageError {
    let root = RelPath::root();
    match err {
        Errno::ACCESS | Errno::PERM => StorageError::PermissionDenied(root),
        Errno::NOSPC | Errno::DQUOT => StorageError::NoSpace(root),
        _ => StorageError::Io {
            path: root,
            source: io::Error::new(
                io::Error::from(err).kind(),
                format!("{STAGING_DIR} is not a usable directory: {err}"),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_both_names() {
        for (name, n, expected) in [
            ("Film.mkv", 1, "Film (1).mkv"),
            ("archive.tar.gz", 2, "archive.tar (2).gz"),
            (".bashrc", 1, ".bashrc (1)"),
            ("noext", 1, "noext (1)"),
            ("a.", 1, "a. (1)"),
            ("Film.mkv", 999, "Film (999).mkv"),
        ] {
            assert_eq!(keep_both_name(name, n), expected, "{name:?} {n}");
        }
    }

    #[test]
    fn prefixes_of_a_path() {
        let rel = |raw| RelPath::parse(raw).unwrap();
        assert_eq!(
            prefixes(&rel("a/b/c")),
            [rel("a"), rel("a/b"), rel("a/b/c")]
        );
        assert!(prefixes(&RelPath::root()).is_empty());
    }
}
