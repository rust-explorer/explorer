use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use cap_std::fs::{Dir, Metadata};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use rustix::io::Errno;

use super::read::fs_path;
use super::{InvalidPath, RelPath, StorageError, Volume};

/// Folders this deep or deeper give up their descriptor while their sub-folders are removed,
/// and are reopened through `..` afterwards, so that open descriptors don't grow with depth.
const KEEP_OPEN_DEPTH: usize = 64;
/// A folder that is still not empty after this many listings (entries keep appearing) stays.
const LISTINGS: u32 = 3;

impl Volume {
    /// Removes a file, a symlink (never its target) or a whole folder tree.
    pub fn delete(&self, path: &RelPath) -> Result<(), StorageError> {
        let (dir, name, meta) = self.locate(path)?;
        let removed = if meta.is_dir() {
            remove_tree(&dir, name)
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

/// A folder being emptied: `fd` is `None` while a deeper one is worked on past
/// [`KEEP_OPEN_DEPTH`]; `subdirs` are the names still to remove.
struct Level {
    fd: Option<OwnedFd>,
    id: (u64, u64),
    name: CString,
    subdirs: Vec<CString>,
    listings: u32,
}

/// Removes the folder `name` of `parent` and everything in it, depth-first without recursion.
/// Symlinks are removed, never followed. A folder is only entered, emptied and removed through
/// a descriptor known by device and inode to be the one that was found there, so a tree that
/// changes meanwhile makes the delete fail rather than reach anything else.
pub(super) fn remove_tree(parent: &Dir, name: &str) -> io::Result<()> {
    let name = CString::new(name)?;
    let top = open_dir_at(parent.as_fd(), &name)?;
    let id = id_of(&rustix::fs::fstat(&top)?);
    let mut levels = vec![Level {
        fd: Some(top),
        id,
        name,
        subdirs: Vec::new(),
        listings: 0,
    }];
    let mut entered = HashSet::from([id]);
    while let Some(deepest) = levels.len().checked_sub(1) {
        let level = &mut levels[deepest];
        if level.listings == 0 {
            list(level)?;
            continue;
        }
        if let Some(sub) = level.subdirs.pop() {
            let fd = level.fd.as_ref().expect("the deepest folder is open");
            let child = match open_dir_at(fd.as_fd(), &sub) {
                Ok(child) => child,
                Err(Errno::NOENT) => continue,
                // Replaced by a file or a symlink since it was listed.
                Err(Errno::NOTDIR | Errno::LOOP) => {
                    unlink(fd.as_fd(), &sub)?;
                    continue;
                }
                Err(err) => return Err(err.into()),
            };
            let child_id = id_of(&rustix::fs::fstat(&child)?);
            if !entered.insert(child_id) {
                return Err(io::Error::other("a folder is mounted inside itself"));
            }
            if deepest >= KEEP_OPEN_DEPTH {
                level.fd = None;
            }
            levels.push(Level {
                fd: Some(child),
                id: child_id,
                name: sub,
                subdirs: Vec::new(),
                listings: 0,
            });
            continue;
        }

        let removed = match deepest.checked_sub(1) {
            None => remove_dir_checked(parent.as_fd(), &levels[0]),
            Some(above) => {
                let (upper, lower) = levels.split_at_mut(deepest);
                let (parent_level, level) = (&mut upper[above], &lower[0]);
                reopen(parent_level, level)?;
                let fd = parent_level.fd.as_ref().expect("reopened");
                remove_dir_checked(fd.as_fd(), level)
            }
        };
        match removed {
            Ok(()) => {
                let done = levels.pop().expect("a folder was removed");
                entered.remove(&done.id);
            }
            Err(err)
                if is_not_empty(&err) && levels.last().is_some_and(|l| l.listings < LISTINGS) =>
            {
                list(levels.last_mut().expect("still there"))?;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Removes the files and symlinks of `level`'s folder and notes its sub-folders.
fn list(level: &mut Level) -> io::Result<()> {
    level.listings += 1;
    let fd = level.fd.as_ref().expect("a listed folder is open");
    for entry in rustix::fs::Dir::read_from(fd)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let is_dir = match entry.file_type() {
            FileType::Directory => true,
            FileType::Unknown => match rustix::fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => FileType::from_raw_mode(stat.st_mode) == FileType::Directory,
                Err(Errno::NOENT) => continue,
                Err(err) => return Err(err.into()),
            },
            _ => false,
        };
        if is_dir {
            level.subdirs.push(name.to_owned());
            continue;
        }
        match rustix::fs::unlinkat(fd, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => {}
            // Became a folder since it was listed.
            Err(Errno::ISDIR | Errno::PERM) if is_dir_at(fd.as_fd(), name) => {
                level.subdirs.push(name.to_owned());
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

/// Gives `parent` its descriptor back through `..` of `child`, if it gave it up.
fn reopen(parent: &mut Level, child: &Level) -> io::Result<()> {
    if parent.fd.is_some() {
        return Ok(());
    }
    let child_fd = child.fd.as_ref().expect("the deepest folder is open");
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let fd = rustix::fs::openat(child_fd, c"..", flags, Mode::empty())?;
    if id_of(&rustix::fs::fstat(&fd)?) != parent.id {
        return Err(moved());
    }
    parent.fd = Some(fd);
    Ok(())
}

/// Removes the emptied folder of `level` from `parent`, if that name still leads to it.
fn remove_dir_checked(parent: BorrowedFd<'_>, level: &Level) -> io::Result<()> {
    match rustix::fs::statat(parent, &level.name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if id_of(&stat) == level.id => {}
        Ok(_) | Err(Errno::NOENT) => return Err(moved()),
        Err(err) => return Err(err.into()),
    }
    rustix::fs::unlinkat(parent, &level.name, AtFlags::REMOVEDIR).map_err(Into::into)
}

fn open_dir_at(dir: BorrowedFd<'_>, name: &CStr) -> Result<OwnedFd, Errno> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    rustix::fs::openat(dir, name, flags, Mode::empty())
}

fn unlink(dir: BorrowedFd<'_>, name: &CStr) -> io::Result<()> {
    match rustix::fs::unlinkat(dir, name, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn is_dir_at(dir: BorrowedFd<'_>, name: &CStr) -> bool {
    rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory)
}

fn is_not_empty(err: &io::Error) -> bool {
    [Errno::NOTEMPTY, Errno::EXIST]
        .iter()
        .any(|errno| err.raw_os_error() == Some(errno.raw_os_error()))
}

fn moved() -> io::Error {
    io::Error::other("a folder moved while it was being deleted")
}

// The field types differ between platforms: what is a cast on one is a no-op on another.
#[allow(
    clippy::unnecessary_cast,
    clippy::cast_sign_loss,
    clippy::useless_conversion
)]
fn id_of(stat: &Stat) -> (u64, u64) {
    (stat.st_dev as u64, stat.st_ino as u64)
}
