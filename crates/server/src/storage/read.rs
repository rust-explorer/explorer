use std::io;
use std::time::SystemTime;

use cap_std::fs::{Metadata, MetadataExt as _, OpenOptions, OpenOptionsExt};
use rustix::fs::OFlags;

use super::error::is_escape;
use super::path::{STAGING_DIR, is_staging_name};
use super::write::prefixes;
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

pub(super) struct Entry {
    pub(super) meta: EntryMeta,
    pub(super) path: Option<RelPath>,
}

/// A directory or file by device and inode, whatever path or symlink leads to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity(u64, u64);

impl Identity {
    pub(super) fn of(meta: &Metadata) -> Self {
        Self(meta.dev(), meta.ino())
    }
}

/// What makes an entry part of the staging area: being the staging directory, or bearing its
/// name in the volume root, however either is reached.
pub(super) struct Reserved {
    root: Identity,
    staging: Option<Identity>,
}

impl Reserved {
    pub(super) fn hides(&self, parent: Identity, name: &str, entry: Option<Identity>) -> bool {
        (parent == self.root && is_staging_name(name)) || (entry.is_some() && entry == self.staging)
    }
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

pub(super) fn fs_path(path: &RelPath) -> &str {
    if path.is_root() { "." } else { path.as_str() }
}

/// Every path is resolved from the volume root with symlinks followed, so cap-std refuses any
/// resolution that leaves the volume, whichever directory the symlink lives in.
impl Volume {
    /// Entries sorted by name, bytewise.
    pub fn list(&self, dir: &RelPath) -> Result<Vec<EntryMeta>, StorageError> {
        self.list_while(dir, &|| true)
    }

    /// Like [`Volume::list`], failing with `Cancelled` as soon as `go_on` answers `false`; it is
    /// asked before each entry.
    pub fn list_while(
        &self,
        dir: &RelPath,
        go_on: &dyn Fn() -> bool,
    ) -> Result<Vec<EntryMeta>, StorageError> {
        self.refuse_reserved(dir)?;
        Ok(self
            .entries_while(dir, go_on)?
            .into_iter()
            .map(|entry| entry.meta)
            .collect())
    }

    pub fn stat(&self, path: &RelPath) -> Result<EntryMeta, StorageError> {
        self.refuse_reserved(path)?;
        let meta = match self.in_checked_parent(path)? {
            Some((_, _, meta)) => meta,
            None => self.metadata(path)?,
        };
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
        self.refuse_reserved(path)?;
        // Non-blocking so that a FIFO swapped in after the check cannot hang the open; it has no
        // effect on regular files. Opened from its checked parent unless it is a symlink.
        let mut flags = OFlags::NONBLOCK | OFlags::NOCTTY;
        let mut options = OpenOptions::new();
        options.read(true);
        let opened = if let Some((parent, name, meta)) = self.in_checked_parent(path)? {
            check(&meta)?;
            flags |= OFlags::NOFOLLOW;
            options.custom_flags(flags.bits().cast_signed());
            parent.open_with(name, &options)
        } else {
            check(&self.metadata(path)?)?;
            options.custom_flags(flags.bits().cast_signed());
            self.root.open_with(fs_path(path), &options)
        };
        let file = opened.map_err(|err| StorageError::from_io(path, err))?;
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
        self.walk_counting(dir).map(|(items, _)| items)
    }

    /// Like [`Volume::walk`], with the number of entries it skipped.
    pub fn walk_counting(&self, dir: &RelPath) -> Result<(Vec<WalkItem>, u64), StorageError> {
        self.refuse_reserved(dir)?;
        let start = self.metadata(dir)?;
        if !start.is_dir() {
            return Err(StorageError::NotADirectory(dir.clone()));
        }
        let mut pending = Vec::new();
        let mut skipped = 0;
        self.push_children(dir, &mut pending, &mut skipped)?;
        let mut items = Vec::new();
        while let Some((path, meta)) = pending.pop() {
            if meta.kind == EntryKind::File {
                items.push(WalkItem::File {
                    path,
                    size: meta.size.unwrap_or(0),
                    modified: meta.modified,
                });
            } else {
                self.push_children(&path, &mut pending, &mut skipped)?;
                items.push(WalkItem::Dir(path));
            }
        }
        Ok((items, skipped))
    }

    pub fn space(&self) -> io::Result<Space> {
        let stat = rustix::fs::fstatvfs(&self.root)?;
        Ok(Space {
            total: stat.f_blocks.saturating_mul(stat.f_frsize),
            available: stat.f_bavail.saturating_mul(stat.f_frsize),
        })
    }

    /// Refuses, as not found, a path that leads to or into the staging directory, which a
    /// symlink to the volume root or to the directory itself would otherwise allow.
    pub(super) fn refuse_reserved(&self, path: &RelPath) -> Result<(), StorageError> {
        let reserved = self.reserved()?;
        let mut parent = reserved.root;
        for prefix in prefixes(path) {
            let name = prefix.file_name().unwrap_or_default();
            let entry = self
                .root
                .metadata(prefix.as_str())
                .ok()
                .map(|m| Identity::of(&m));
            if reserved.hides(parent, name, entry) {
                return Err(StorageError::NotFound(path.clone()));
            }
            match entry {
                Some(entry) => parent = entry,
                // Nothing further down exists; the operation reports its own error.
                None => return Ok(()),
            }
        }
        Ok(())
    }

    pub(super) fn reserved(&self) -> Result<Reserved, StorageError> {
        let root = self
            .root
            .dir_metadata()
            .map_err(|err| StorageError::from_io(&RelPath::root(), err))?;
        let staging = self
            .root
            .symlink_metadata(STAGING_DIR)
            .ok()
            .filter(Metadata::is_dir)
            .map(|meta| Identity::of(&meta));
        Ok(Reserved {
            root: Identity::of(&root),
            staging,
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

    pub(super) fn entries(&self, dir: &RelPath) -> Result<Vec<Entry>, StorageError> {
        self.entries_while(dir, &|| true)
    }

    /// Like [`Volume::entries`], failing with `Cancelled` as soon as `go_on` answers `false`; it
    /// is asked before each entry.
    pub(super) fn entries_while(
        &self,
        dir: &RelPath,
        go_on: &dyn Fn() -> bool,
    ) -> Result<Vec<Entry>, StorageError> {
        let map_err = |err| StorageError::from_io(dir, err);
        let handle = self.root.open_dir(fs_path(dir)).map_err(map_err)?;
        let reserved = self.reserved()?;
        let listed = Identity::of(&handle.dir_metadata().map_err(map_err)?);
        if reserved.staging == Some(listed) {
            return Err(StorageError::NotFound(dir.clone()));
        }
        let mut entries = Vec::new();
        for entry in handle.entries().map_err(map_err)? {
            if !go_on() {
                return Err(StorageError::Cancelled);
            }
            let file_name = entry.map_err(map_err)?.file_name();
            let name = file_name.to_string_lossy().into_owned();
            if reserved.hides(listed, &name, None) {
                continue;
            }
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
            if let Some((_, meta)) = &resolved
                && reserved.hides(listed, &name, Some(Identity::of(meta)))
            {
                continue;
            }
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

    pub(super) fn is_real_dir(&self, path: &RelPath) -> bool {
        self.root
            .symlink_metadata(path.as_str())
            .is_ok_and(|meta| meta.is_dir())
    }

    fn push_children(
        &self,
        dir: &RelPath,
        pending: &mut Vec<(RelPath, EntryMeta)>,
        skipped: &mut u64,
    ) -> Result<(), StorageError> {
        for entry in self.entries(dir)?.into_iter().rev() {
            match entry.path {
                Some(path)
                    if entry.meta.kind == EntryKind::File
                        || (entry.meta.kind == EntryKind::Dir && self.is_real_dir(&path)) =>
                {
                    pending.push((path, entry.meta));
                }
                _ => {
                    *skipped += 1;
                    tracing::debug!(
                        volume = self.name(),
                        dir = dir.as_str(),
                        name = entry.meta.name,
                        "walk skips a symlinked directory or an entry that is neither a file nor a directory"
                    );
                }
            }
        }
        Ok(())
    }
}
