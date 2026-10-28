use std::ffi::CString;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;

use rustix::fs::{FileType, Mode, OFlags, Stat};

/// Folders this deep or deeper don't keep their parent open while they wait: they are reopened
/// from the scanned folder one name at a time, so that open descriptors don't grow with depth.
pub(super) const KEEP_PARENT_DEPTH: usize = 64;

/// What a scan needs from a `stat`.
#[derive(Clone, Copy)]
pub(super) struct Measured {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub bytes: u64,
    pub is_dir: bool,
    pub is_file: bool,
}

impl Measured {
    // The field types differ between platforms: what is a cast on one is a no-op on another.
    #[allow(
        clippy::unnecessary_cast,
        clippy::cast_sign_loss,
        clippy::cast_lossless,
        clippy::useless_conversion
    )]
    pub fn of(stat: &Stat) -> Self {
        let kind = FileType::from_raw_mode(stat.st_mode);
        Self {
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
            nlink: stat.st_nlink as u64,
            bytes: (stat.st_blocks as u64).saturating_mul(512),
            is_dir: kind == FileType::Directory,
            is_file: kind == FileType::RegularFile,
        }
    }

    pub fn id(self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

/// A folder to read: the scanned folder, a sub-folder to open from its parent, or a deep one to
/// open along its path from the scanned folder.
pub(super) enum Source {
    Start,
    At {
        parent: Arc<OwnedFd>,
        name: CString,
        ino: u64,
    },
    Path {
        ino: u64,
    },
}

impl Source {
    /// `path`: the folder's names from the scanned folder down, `depth` of them.
    pub fn sub(parent: Arc<OwnedFd>, name: CString, ino: u64, depth: usize) -> Self {
        if depth >= KEEP_PARENT_DEPTH {
            Self::Path { ino }
        } else {
            Self::At { parent, name, ino }
        }
    }

    /// Opens the folder for listing, never following a symlink; a sub-folder only if it is still
    /// the one that was measured. `start` is the scanned folder, `path` the folder's names from
    /// it.
    pub fn open(&self, start: &OwnedFd, path: &[CString], dev: u64) -> Option<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let (fd, ino) = match self {
            // cap-std opens folders with O_PATH on Linux, which cannot be listed: reopen it.
            Self::Start => return rustix::fs::openat(start, c".", flags, Mode::empty()).ok(),
            Self::At { parent, name, ino } => (
                rustix::fs::openat(parent.as_fd(), name.as_c_str(), flags, Mode::empty()).ok()?,
                *ino,
            ),
            Self::Path { ino } => {
                let (first, rest) = path.split_first()?;
                let mut fd =
                    rustix::fs::openat(start, first.as_c_str(), flags, Mode::empty()).ok()?;
                for name in rest {
                    fd = rustix::fs::openat(&fd, name.as_c_str(), flags, Mode::empty()).ok()?;
                }
                (fd, *ino)
            }
        };
        let measured = Measured::of(&rustix::fs::fstat(&fd).ok()?);
        (measured.id() == (dev, ino)).then_some(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_folders_do_not_hold_their_parent() {
        let dir = tempfile::tempdir().unwrap();
        let parent = Arc::new(OwnedFd::from(std::fs::File::open(dir.path()).unwrap()));
        let name = CString::new("x").unwrap();

        let shallow = Source::sub(Arc::clone(&parent), name.clone(), 1, KEEP_PARENT_DEPTH - 1);
        assert!(matches!(shallow, Source::At { .. }));
        assert_eq!(Arc::strong_count(&parent), 2);
        let deep = Source::sub(Arc::clone(&parent), name, 1, KEEP_PARENT_DEPTH);
        assert!(matches!(deep, Source::Path { ino: 1 }));
        assert_eq!(Arc::strong_count(&parent), 2);
    }
}
