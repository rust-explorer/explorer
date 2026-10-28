use std::io;
use std::time::SystemTime;

use cap_std::fs::Dir;
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use shared::api::conflict::ConflictPolicy;

use super::path::STAGING_DIR;
use super::read::fs_path;
use super::rename::rename_noreplace;
use super::{RelPath, StagedId, Staging, StorageError, Volume};

pub(super) const KEEP_BOTH_MAX: u32 = 999;
/// A Replace whose target keeps appearing and vanishing gives up after this many tries.
const REPLACE_ATTEMPTS: usize = 3;

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
    /// Creates one folder under an existing parent; anything already at `dir` is an error.
    pub fn create_dir(&self, dir: &RelPath) -> Result<(), StorageError> {
        self.refuse_reserved(dir)?;
        self.root
            .create_dir(fs_path(dir))
            .map_err(|err| StorageError::from_io(dir, err))
    }

    pub fn create_dir_all(&self, dir: &RelPath) -> Result<(), StorageError> {
        self.create_missing_dirs(dir).map(drop)
    }

    /// Like [`Volume::create_dir_all`], returning the folders it created, outermost first. A
    /// dangling symlink on the way is not a folder.
    fn create_missing_dirs(&self, dir: &RelPath) -> Result<Vec<RelPath>, StorageError> {
        self.refuse_reserved(dir)?;
        let mut created = Vec::new();
        let not_a_dir = |prefix: RelPath| {
            if prefix == *dir {
                StorageError::AlreadyExists(prefix)
            } else {
                StorageError::NotADirectory(prefix)
            }
        };
        for prefix in prefixes(dir) {
            let result = match self.is_dir_at(&prefix) {
                Ok(Some(true)) => Ok(()),
                Ok(Some(false)) => Err(not_a_dir(prefix.clone())),
                Ok(None) => match self.root.create_dir(prefix.as_str()) {
                    Ok(()) => {
                        created.push(prefix.clone());
                        Ok(())
                    }
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                        match self.is_dir_at(&prefix) {
                            Ok(Some(true)) => Ok(()),
                            Ok(_) => Err(not_a_dir(prefix.clone())),
                            Err(err) => Err(err),
                        }
                    }
                    Err(err) => Err(StorageError::from_io(&prefix, err)),
                },
                Err(err) => Err(err),
            };
            if let Err(err) = result {
                self.remove_created_dirs(&created);
                return Err(err);
            }
        }
        Ok(created)
    }

    /// Removes folders a failed operation created, deepest first, as long as they are empty.
    fn remove_created_dirs(&self, created: &[RelPath]) {
        for dir in created.iter().rev() {
            if let Err(err) = self.root.remove_dir(dir.as_str()) {
                tracing::debug!(volume = self.name(), path = dir.as_str(), error = %err, "created folder left in place");
                return;
            }
        }
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
                None => {
                    return match self.root.symlink_metadata(prefix.as_str()) {
                        Ok(_) => Err(StorageError::NotADirectory(prefix)),
                        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                        Err(err) => Err(StorageError::from_io(&prefix, err)),
                    };
                }
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
    pub fn commit_staged(
        &self,
        staging: &Staging,
        id: &StagedId,
        dest: &RelPath,
        conflict: ConflictPolicy,
        modified: Option<SystemTime>,
    ) -> Result<RelPath, StorageError> {
        let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
            return Err(StorageError::IsADirectory(dest.clone()));
        };
        self.refuse_reserved(dest)?;
        let created = self.create_missing_dirs(&parent)?;
        let placed = self.place_staged(staging, id, &parent, name, conflict, modified);
        if placed.is_err() {
            self.remove_created_dirs(&created);
        }
        placed
    }

    fn place_staged(
        &self,
        staging: &Staging,
        id: &StagedId,
        parent: &RelPath,
        name: &str,
        conflict: ConflictPolicy,
        modified: Option<SystemTime>,
    ) -> Result<RelPath, StorageError> {
        let parent_dir = self
            .root
            .open_dir(fs_path(parent))
            .map_err(|err| StorageError::from_io(parent, err))?;

        let dest = parent.join(name)?;
        let staged_err = |err| StorageError::from_io(&dest, err);
        let file = staging.open_staged(id).map_err(staged_err)?;
        if let Some(modified) = modified {
            file.set_modified(modified).map_err(staged_err)?;
        }
        file.sync_all().map_err(staged_err)?;
        drop(file);

        let placed = self.place(
            &staging.dir,
            id.as_str(),
            &parent_dir,
            parent,
            name,
            conflict,
        )?;
        if let Err(err) = sync_dir(&parent_dir) {
            tracing::warn!(volume = self.name(), path = parent.as_str(), error = %err, "cannot sync the folder of a committed file");
        }
        Ok(placed)
    }

    /// Renames `from_name` to `to_name`, or under `Fail` and `KeepBoth` to the first free
    /// `Name (n)` when that is taken: those never overwrite, even a target that appears meanwhile.
    pub(super) fn place(
        &self,
        from_dir: &Dir,
        from_name: &str,
        to_dir: &Dir,
        to_parent: &RelPath,
        to_name: &str,
        conflict: ConflictPolicy,
    ) -> Result<RelPath, StorageError> {
        let dest = to_parent.join(to_name)?;
        if conflict == ConflictPolicy::Replace {
            return replace(from_dir, from_name, to_dir, to_name, dest);
        }

        for n in 0..=KEEP_BOTH_MAX {
            let candidate = if n == 0 {
                to_name.to_owned()
            } else {
                keep_both_name(to_name, n)
            };
            let Ok(target) = to_parent.join(&candidate) else {
                break;
            };
            match rename_noreplace(from_dir, from_name, to_dir, &candidate) {
                Ok(()) => {
                    if n > 0 && conflict == ConflictPolicy::Fail {
                        tracing::info!(
                            volume = self.name(),
                            path = dest.as_str(),
                            final_path = target.as_str(),
                            "destination appeared meanwhile; keeping both"
                        );
                    }
                    return Ok(target);
                }
                Err(Errno::EXIST) => {}
                Err(err) => return Err(rename_error(&target, err)),
            }
        }
        Err(StorageError::AlreadyExists(dest))
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

/// Replaces a file in one rename. With nothing there, the rename never replaces, so that a
/// folder created meanwhile stays.
fn replace(
    from_dir: &Dir,
    from_name: &str,
    to_dir: &Dir,
    to_name: &str,
    dest: RelPath,
) -> Result<RelPath, StorageError> {
    for _ in 0..REPLACE_ATTEMPTS {
        match to_dir.symlink_metadata(to_name) {
            Ok(_) => {
                if is_dir_in(to_dir, to_name) || is_dir_in(from_dir, from_name) {
                    return Err(StorageError::IsADirectory(dest));
                }
                rustix::fs::renameat(from_dir, from_name, to_dir, to_name)
                    .map_err(|err| rename_error(&dest, err))?;
                return Ok(dest);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                match rename_noreplace(from_dir, from_name, to_dir, to_name) {
                    Ok(()) => return Ok(dest),
                    Err(Errno::EXIST) => {}
                    Err(err) => return Err(rename_error(&dest, err)),
                }
            }
            Err(err) => return Err(StorageError::from_io(&dest, err)),
        }
    }
    Err(StorageError::AlreadyExists(dest))
}

/// cap-std may hold folders as `O_PATH` descriptors, which cannot be synced: reopen it first.
fn sync_dir(dir: &Dir) -> io::Result<()> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(dir, c".", flags, Mode::empty())?;
    rustix::fs::fsync(fd)?;
    Ok(())
}

/// Follows a symlink, like the read operations; a dangling one is not a folder.
fn is_dir_in(dir: &Dir, name: &str) -> bool {
    dir.metadata(name).is_ok_and(|meta| meta.is_dir())
}

/// Both sides are plain names under open `Dir` handles, so `EXDEV` here is a mount point nested
/// in the volume, not an escape.
fn rename_error(path: &RelPath, err: Errno) -> StorageError {
    match err {
        Errno::XDEV => StorageError::CrossDevice(path.clone()),
        _ => StorageError::from_io(path, err.into()),
    }
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
    fn rename_einval_stays_an_io_error() {
        let path = RelPath::parse("a.txt").unwrap();
        assert!(matches!(
            rename_error(&path, Errno::INVAL),
            StorageError::Io { .. }
        ));
        assert!(matches!(
            rename_error(&path, Errno::XDEV),
            StorageError::CrossDevice(_)
        ));
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
