use std::io;
use std::path::Path;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use rustix::fs::{Access, AtFlags};

pub struct Volume {
    name: String,
    #[expect(dead_code, reason = "read by the storage operations")]
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
            let Some(name) = file_name.to_str() else {
                tracing::warn!(name = ?file_name, "skipping volume with a non UTF-8 name");
                continue;
            };
            match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => {}
                Ok(_) => continue,
                Err(err) => {
                    tracing::warn!(volume = name, error = %err, "skipping unreadable volume");
                    continue;
                }
            }
            let root = match entry.open_dir() {
                Ok(root) => root,
                Err(err) => {
                    tracing::warn!(volume = name, error = %err, "skipping unreadable volume");
                    continue;
                }
            };
            let read_only =
                rustix::fs::accessat(&root, ".", Access::WRITE_OK, AtFlags::EACCESS).is_err();
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
