use std::io;

use cap_std::fs::{Metadata, MetadataExt as _};

use crate::storage::read::{Identity, fs_path};
use crate::storage::{InvalidPath, RelPath, StorageError, Volume};

/// A file's content as found at one moment: where it lives (device, inode), its size and
/// modification time and, unless left out, its change time, which a write, a `chmod` or a
/// rename onto it also moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: Option<(i64, i64)>,
}

impl Stamp {
    /// A source file as it was opened for copying.
    pub fn source(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.size(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: Some((meta.ctime(), meta.ctime_nsec())),
        }
    }

    /// A file a copy placed: renaming it into place moved its change time, which is left out.
    pub fn placed(meta: &std::fs::Metadata) -> Self {
        Self {
            ctime: None,
            ..Self::source(meta)
        }
    }

    fn matches(&self, meta: &Metadata) -> bool {
        meta.is_file()
            && (meta.dev(), meta.ino(), meta.size()) == (self.dev, self.ino, self.size)
            && (meta.mtime(), meta.mtime_nsec()) == self.mtime
            && self
                .ctime
                .is_none_or(|ctime| (meta.ctime(), meta.ctime_nsec()) == ctime)
    }
}

/// What a copy or a move expects to find at a path it is about to remove.
#[derive(Debug, Clone, Copy)]
pub(super) enum Expect {
    File(Stamp),
    Dir(Identity),
}

impl Expect {
    fn matches(&self, meta: &Metadata) -> bool {
        match self {
            Self::File(stamp) => stamp.matches(meta),
            Self::Dir(id) => meta.is_dir() && Identity::of(meta) == *id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Removed {
    Yes,
    /// Nothing is there any more.
    Gone,
    /// Something else is there now; it stays.
    Changed,
    /// The expected folder, but it holds something: it stays.
    NotEmpty,
}

/// Removes the entry at `path` only if it is still `expect`. With `through_link`, a symlink
/// there is compared by what it leads to, and then only the link goes.
///
/// The check and the removal go through one handle on the parent folder, so that what is
/// checked is what is removed, short of a change between the two calls.
pub(super) fn remove_if(
    volume: &Volume,
    path: &RelPath,
    expect: Expect,
    through_link: bool,
) -> Result<Removed, StorageError> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(InvalidPath {
            reason: "the volume root can't be removed",
        }
        .into());
    };
    let gone_or = |err: io::Error| {
        if err.kind() == io::ErrorKind::NotFound {
            Ok(Removed::Gone)
        } else {
            Err(StorageError::from_io(path, err))
        }
    };
    let dir = match volume.root.open_dir(fs_path(&parent)) {
        Ok(dir) => dir,
        Err(err) => return gone_or(err),
    };
    let meta = match dir.symlink_metadata(name) {
        Ok(meta) => meta,
        Err(err) => return gone_or(err),
    };
    let found = if meta.is_symlink() && through_link {
        volume.root.metadata(path.as_str()).ok()
    } else {
        Some(meta.clone())
    };
    if !found.is_some_and(|found| expect.matches(&found)) {
        return Ok(Removed::Changed);
    }
    let removed = if meta.is_dir() {
        dir.remove_dir(name)
    } else {
        dir.remove_file(name)
    };
    match removed {
        Ok(()) => Ok(Removed::Yes),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(Removed::NotEmpty),
        Err(err) => gone_or(err),
    }
}
