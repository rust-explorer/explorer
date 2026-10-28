use std::io;
use std::time::SystemTime;

use cap_std::fs::{Metadata, OpenOptions, OpenOptionsExt};
use rustix::fs::OFlags;

use super::error::is_escape;
use super::{RelPath, StorageError, Volume};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Other,
}

#[derive(Debug, Clone)]
pub struct EntryMeta {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

#[derive(Debug)]
pub enum WalkItem {
    Dir(RelPath),
    File {
        path: RelPath,
        size: u64,
        modified: Option<SystemTime>,
    },
}

struct Entry {
    meta: EntryMeta,
    path: Option<RelPath>,
}

impl EntryMeta {
    fn from_metadata(name: String, meta: &Metadata) -> Self {
        let kind = if meta.is_dir() {
            EntryKind::Dir
        } else if meta.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        Self {
            name,
            kind,
            size: (kind == EntryKind::File).then(|| meta.len()),
            modified: meta
                .modified()
                .ok()
                .map(cap_std::time::SystemTime::into_std),
        }
    }

    fn other(name: String) -> Self {
        Self {
            name,
            kind: EntryKind::Other,
            size: None,
            modified: None,
        }
    }
}

fn fs_path(path: &RelPath) -> &str {
    if path.is_root() { "." } else { path.as_str() }
}

/// Every path is resolved from the volume root with symlinks followed, so cap-std refuses any
/// resolution that leaves the volume, whichever directory the symlink lives in.
impl Volume {
    /// Entries sorted by name, bytewise.
    pub fn list(&self, dir: &RelPath) -> Result<Vec<EntryMeta>, StorageError> {
        Ok(self
            .entries(dir)?
            .into_iter()
            .map(|entry| entry.meta)
            .collect())
    }

    pub fn stat(&self, path: &RelPath) -> Result<EntryMeta, StorageError> {
        let meta = self.metadata(path)?;
        Ok(EntryMeta::from_metadata(self.entry_name(path), &meta))
    }

    pub fn open_file(&self, path: &RelPath) -> Result<(std::fs::File, EntryMeta), StorageError> {
        let check = |meta: &Metadata| {
            if meta.is_dir() {
                Err(StorageError::IsADirectory(path.clone()))
            } else if meta.is_file() {
                Ok(())
            } else {
                Err(StorageError::NotFound(path.clone()))
            }
        };
        check(&self.metadata(path)?)?;
        // Non-blocking so that a FIFO swapped in after the check cannot hang the open; it has no
        // effect on regular files.
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags((OFlags::NONBLOCK | OFlags::NOCTTY).bits().cast_signed());
        let file = self
            .root
            .open_with(fs_path(path), &options)
            .map_err(|err| StorageError::from_io(path, err))?;
        let meta = file
            .metadata()
            .map_err(|err| StorageError::from_io(path, err))?;
        check(&meta)?;
        let entry = EntryMeta::from_metadata(self.entry_name(path), &meta);
        Ok((file.into_std(), entry))
    }

    /// Depth-first, each directory before its contents. Entries that are neither files nor
    /// directories are skipped, and so are symlinks to directories, which also rules out cycles.
    pub fn walk(&self, dir: &RelPath) -> Result<Vec<WalkItem>, StorageError> {
        let start = self.metadata(dir)?;
        if !start.is_dir() {
            return Err(StorageError::NotADirectory(dir.clone()));
        }
        let mut pending = Vec::new();
        self.push_children(dir, &mut pending)?;
        let mut items = Vec::new();
        while let Some((path, meta)) = pending.pop() {
            if meta.kind == EntryKind::File {
                items.push(WalkItem::File {
                    path,
                    size: meta.size.unwrap_or(0),
                    modified: meta.modified,
                });
            } else {
                self.push_children(&path, &mut pending)?;
                items.push(WalkItem::Dir(path));
            }
        }
        Ok(items)
    }

    pub fn space(&self) -> io::Result<Space> {
        let stat = rustix::fs::fstatvfs(&self.root)?;
        Ok(Space {
            total: stat.f_blocks.saturating_mul(stat.f_frsize),
            available: stat.f_bavail.saturating_mul(stat.f_frsize),
        })
    }

    fn metadata(&self, path: &RelPath) -> Result<Metadata, StorageError> {
        self.root
            .metadata(fs_path(path))
            .map_err(|err| StorageError::from_io(path, err))
    }

    fn entry_name(&self, path: &RelPath) -> String {
        path.file_name().unwrap_or(self.name()).to_owned()
    }

    fn entries(&self, dir: &RelPath) -> Result<Vec<Entry>, StorageError> {
        let map_err = |err| StorageError::from_io(dir, err);
        let handle = self.root.open_dir(fs_path(dir)).map_err(map_err)?;
        let mut entries = Vec::new();
        for entry in handle.entries().map_err(map_err)? {
            let file_name = entry.map_err(map_err)?.file_name();
            let name = file_name.to_string_lossy().into_owned();
            let resolved = file_name
                .to_str()
                .and_then(|name| dir.join(name).ok())
                .and_then(|path| match self.root.metadata(path.as_str()) {
                    Ok(meta) => Some((path, meta)),
                    Err(err) => {
                        self.log_entry_error(&path, &err);
                        None
                    }
                });
            let entry = match resolved {
                Some((path, meta)) => Entry {
                    meta: EntryMeta::from_metadata(name, &meta),
                    path: Some(path),
                },
                None => Entry {
                    meta: EntryMeta::other(name),
                    path: None,
                },
            };
            entries.push(entry);
        }
        entries.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
        Ok(entries)
    }

    /// Broken and escaping symlinks are expected; anything else is worth a warning.
    fn log_entry_error(&self, path: &RelPath, err: &io::Error) {
        if err.kind() != io::ErrorKind::NotFound && !is_escape(err) {
            tracing::warn!(
                volume = self.name(),
                path = path.as_str(),
                error = %err,
                "cannot read entry metadata"
            );
        }
    }

    fn is_real_dir(&self, path: &RelPath) -> bool {
        self.root
            .symlink_metadata(path.as_str())
            .is_ok_and(|meta| meta.is_dir())
    }

    fn push_children(
        &self,
        dir: &RelPath,
        pending: &mut Vec<(RelPath, EntryMeta)>,
    ) -> Result<(), StorageError> {
        for entry in self.entries(dir)?.into_iter().rev() {
            match entry.path {
                Some(path)
                    if entry.meta.kind == EntryKind::File
                        || (entry.meta.kind == EntryKind::Dir && self.is_real_dir(&path)) =>
                {
                    pending.push((path, entry.meta));
                }
                _ => tracing::debug!(
                    volume = self.name(),
                    dir = dir.as_str(),
                    name = entry.meta.name,
                    "walk skips a symlinked directory or an entry that is neither a file nor a directory"
                ),
            }
        }
        Ok(())
    }
}
