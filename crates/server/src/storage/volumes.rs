use std::io;
use std::path::Path;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use rustix::fs::{Access, AtFlags};
use rustix::io::Errno;

/// A mounted volume. Its methods are split by concern: reading in `read.rs`, writing and uploads
/// in `write.rs`, renaming in `rename.rs`.
pub struct Volume {
    name: String,
    pub(super) root: Dir,
    read_only: bool,
}

impl Volume {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn read_only(&self) -> bool {
        self.read_only
    }
}

pub struct Volumes {
    volumes: Vec<Volume>,
}

impl Volumes {
    /// Fails only when `data_dir` itself cannot be read; unusable entries are skipped with a warning.
    pub fn discover(data_dir: &Path) -> io::Result<Self> {
        let parent = Dir::open_ambient_dir(data_dir, ambient_authority())?;
        let mut volumes = Vec::new();
        for entry in parent.entries()? {
            let entry = entry?;
            let file_name = entry.file_name();
            match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => {}
                Ok(_) => continue,
                Err(err) => {
                    tracing::warn!(volume = ?file_name, error = %err, "skipping unreadable volume");
                    continue;
                }
            }
            let Some(name) = file_name.to_str() else {
                tracing::warn!(name = ?file_name, "skipping volume with a non UTF-8 name");
                continue;
            };
            let root = match entry.open_dir() {
                Ok(root) => root,
                Err(err) => {
                    tracing::warn!(volume = name, error = %err, "skipping unreadable volume");
                    continue;
                }
            };
            let read_only = is_read_only(name, &root);
            volumes.push(Volume {
                name: name.to_owned(),
                root,
                read_only,
            });
        }
        volumes.sort_by(|a, b| a.name.cmp(&b.name));
        log_volumes(&volumes, data_dir);
        Ok(Self { volumes })
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Volume> {
        self.volumes.iter().find(|v| v.name == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Volume> {
        self.volumes.iter()
    }
}

fn is_read_only(name: &str, root: &Dir) -> bool {
    match writable(|flags| rustix::fs::accessat(root, ".", Access::WRITE_OK, flags)) {
        Ok(writable) => !writable,
        Err(err) => {
            tracing::warn!(volume = name, error = %err, "cannot check whether the volume is writable");
            true
        }
    }
}

/// Older seccomp profiles reject `faccessat2` (used for `AT_EACCESS`) with `EPERM`; the process is
/// not setuid, so the real-id check is equivalent.
fn writable(access: impl Fn(AtFlags) -> rustix::io::Result<()>) -> rustix::io::Result<bool> {
    let result = match access(AtFlags::EACCESS) {
        Err(Errno::PERM | Errno::NOSYS) => access(AtFlags::empty()),
        other => other,
    };
    match result {
        Ok(()) => Ok(true),
        Err(Errno::ACCESS | Errno::ROFS) => Ok(false),
        Err(err) => Err(err),
    }
}

fn log_volumes(volumes: &[Volume], data_dir: &Path) {
    if volumes.is_empty() {
        tracing::warn!(
            "no volumes found under {}; mount folders as /data/<name>",
            data_dir.display()
        );
    }
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    for volume in volumes {
        tracing::info!(
            volume = volume.name,
            read_only = volume.read_only,
            "volume found"
        );
        if volume.read_only {
            tracing::warn!(
                "volume {} is not writable by uid {uid} gid {gid}; check the mount and the container user:",
                volume.name
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn check(results: &[rustix::io::Result<()>]) -> (rustix::io::Result<bool>, Vec<AtFlags>) {
        let calls = RefCell::new(Vec::new());
        let result = writable(|flags| {
            let mut calls = calls.borrow_mut();
            calls.push(flags);
            results[calls.len() - 1]
        });
        (result, calls.into_inner())
    }

    #[test]
    fn writable_uses_effective_ids_first() {
        assert_eq!(check(&[Ok(())]), (Ok(true), vec![AtFlags::EACCESS]));
        assert_eq!(
            check(&[Err(Errno::ACCESS)]),
            (Ok(false), vec![AtFlags::EACCESS])
        );
        assert_eq!(
            check(&[Err(Errno::ROFS)]),
            (Ok(false), vec![AtFlags::EACCESS])
        );
    }

    #[test]
    fn writable_retries_without_eaccess_when_unsupported() {
        for unsupported in [Errno::PERM, Errno::NOSYS] {
            assert_eq!(
                check(&[Err(unsupported), Ok(())]),
                (Ok(true), vec![AtFlags::EACCESS, AtFlags::empty()])
            );
            assert_eq!(
                check(&[Err(unsupported), Err(Errno::ACCESS)]),
                (Ok(false), vec![AtFlags::EACCESS, AtFlags::empty()])
            );
        }
    }

    #[test]
    fn writable_reports_unexpected_errors() {
        assert_eq!(
            check(&[Err(Errno::IO)]),
            (Err(Errno::IO), vec![AtFlags::EACCESS])
        );
        assert_eq!(
            check(&[Err(Errno::NOSYS), Err(Errno::PERM)]),
            (Err(Errno::PERM), vec![AtFlags::EACCESS, AtFlags::empty()])
        );
    }
}
