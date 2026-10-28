use std::io;

use rustix::io::Errno;

use super::RelPath;

/// The message of cap-std's error for a path that resolves outside its `Dir`.
const ESCAPE_MESSAGE: &str = "a path led outside of the filesystem";

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("not found: {}", .0.as_str())]
    NotFound(RelPath),
    #[error("not a directory: {}", .0.as_str())]
    NotADirectory(RelPath),
    #[error("is a directory: {}", .0.as_str())]
    IsADirectory(RelPath),
    #[error("permission denied: {}", .0.as_str())]
    PermissionDenied(RelPath),
    #[error("already exists: {}", .0.as_str())]
    AlreadyExists(RelPath),
    #[error("no space left for {}", .0.as_str())]
    NoSpace(RelPath),
    #[error("outside the volume: {}", .0.as_str())]
    OutsideVolume(RelPath),
    /// A rename onto another filesystem: a mount nested in the volume.
    #[error("on another filesystem than the volume: {}", .0.as_str())]
    CrossDevice(RelPath),
    #[error("i/o error on {}: {source}", .path.as_str())]
    Io { path: RelPath, source: io::Error },
}

impl StorageError {
    pub(super) fn from_io(path: &RelPath, err: io::Error) -> Self {
        let path = path.clone();
        if is_escape(&err) {
            return Self::OutsideVolume(path);
        }
        if is_no_space(&err) {
            return Self::NoSpace(path);
        }
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path),
            io::ErrorKind::NotADirectory => Self::NotADirectory(path),
            io::ErrorKind::IsADirectory => Self::IsADirectory(path),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            _ => Self::Io { path, source: err },
        }
    }
}

/// `openat2(RESOLVE_BENEATH)` reports an escape as `EXDEV`; cap-std usually rewrites it, but not on
/// every code path.
pub(super) fn is_escape(err: &io::Error) -> bool {
    err.raw_os_error() == Some(Errno::XDEV.raw_os_error())
        || err
            .get_ref()
            .is_some_and(|inner| inner.to_string() == ESCAPE_MESSAGE)
}

fn is_no_space(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::StorageFull
        || [Errno::NOSPC, Errno::DQUOT]
            .iter()
            .any(|errno| err.raw_os_error() == Some(errno.raw_os_error()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(err: io::Error) -> StorageError {
        StorageError::from_io(&RelPath::parse("a/b").unwrap(), err)
    }

    #[test]
    fn maps_io_error_kinds() {
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::NOENT.raw_os_error())),
            StorageError::NotFound(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::ACCESS.raw_os_error())),
            StorageError::PermissionDenied(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::NOTDIR.raw_os_error())),
            StorageError::NotADirectory(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::ISDIR.raw_os_error())),
            StorageError::IsADirectory(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::EXIST.raw_os_error())),
            StorageError::AlreadyExists(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::IO.raw_os_error())),
            StorageError::Io { .. }
        ));
    }

    #[test]
    fn maps_escapes_to_outside_volume() {
        assert!(matches!(
            map(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ESCAPE_MESSAGE
            )),
            StorageError::OutsideVolume(_)
        ));
        assert!(matches!(
            map(io::Error::from_raw_os_error(Errno::XDEV.raw_os_error())),
            StorageError::OutsideVolume(_)
        ));
        assert!(matches!(
            map(io::Error::new(io::ErrorKind::PermissionDenied, "other")),
            StorageError::PermissionDenied(_)
        ));
    }

    #[test]
    fn maps_no_space() {
        for errno in [Errno::NOSPC, Errno::DQUOT] {
            assert!(matches!(
                map(io::Error::from_raw_os_error(errno.raw_os_error())),
                StorageError::NoSpace(_)
            ));
        }
        assert!(matches!(
            map(io::Error::from(io::ErrorKind::StorageFull)),
            StorageError::NoSpace(_)
        ));
    }
}
