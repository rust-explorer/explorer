use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use rustix::io::Errno;

use super::path::{STAGING_DIR, is_staging_name};
use super::read::{EntryKind, fs_path};
use super::write::keep_both_name;
use super::{RelPath, StorageError, Volume};

/// Folders this deep or deeper don't keep their descriptor while their contents are walked:
/// each entry reopens them one name at a time from the nearest folder kept open.
const KEEP_OPEN_DEPTH: usize = 64;

/// One step of a [`TreeWalk`]. `name` is the entry's path from the walked folder, `/`-separated.
#[derive(Debug)]
pub enum TreeItem {
    Dir {
        name: String,
        modified: Option<SystemTime>,
    },
    File {
        name: String,
        file: File,
        size: u64,
        modified: Option<SystemTime>,
    },
    Skipped {
        name: String,
        reason: SkipReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Unreadable,
    LinkToFolder,
    LinkNotFollowed,
    Special,
}

impl SkipReason {
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Unreadable => "cannot be read",
            Self::LinkToFolder => "symbolic link to a folder, not followed",
            Self::LinkNotFollowed => "symbolic link that is broken or leads outside the volume",
            Self::Special => "neither a file nor a folder",
        }
    }
}

struct Frame {
    /// `None` from [`KEEP_OPEN_DEPTH`] down.
    fd: Option<OwnedFd>,
    id: (u64, u64),
    /// Its name in its parent, to reopen it.
    raw_name: CString,
    /// Its path in the walk with a trailing `/`, empty for the walked folder.
    prefix: String,
    /// Its volume path, when every name on the way is a valid `RelPath` component.
    rel: Option<RelPath>,
    /// Left to visit, last first.
    names: Vec<CString>,
    used: HashSet<String>,
}

/// A depth-first walk that yields as it goes, holding one folder listing per level; folders
/// come before their contents. Folders and files are opened by name from their parent without
/// following symlinks, so every name on disk is reached, even one that is not UTF-8, has control
/// characters or lies under a path too long for a `RelPath`. Symlinks to files are followed
/// within the volume; symlinks to folders, special files and whatever cannot be read are
/// reported as skipped. The staging folder is left out. Names given out are made safe for an
/// archive: lossy UTF-8, control characters and backslashes turned into `_`, unique within
/// their folder.
pub struct TreeWalk {
    root_id: (u64, u64),
    staging: Option<(u64, u64)>,
    frames: Vec<Frame>,
    /// The entries of the walked folder were chosen by name: symlinks to folders among them
    /// are followed, as the folder itself would be.
    chosen: bool,
}

/// What visiting one name found.
enum Visit {
    Hidden,
    Item(TreeItem),
    Folder(TreeItem, Frame),
}

impl Volume {
    /// Starts a walk of `dir`: all of it, or with `names` only the entries so named. A folder
    /// that cannot be listed fails here, before anything is yielded.
    pub fn tree_walk(
        &self,
        dir: &RelPath,
        names: Option<&[String]>,
    ) -> Result<TreeWalk, StorageError> {
        self.refuse_reserved(dir)?;
        let map_err = |err: io::Error| StorageError::from_io(dir, err);
        let handle: OwnedFd = self.root.open_dir(fs_path(dir)).map_err(map_err)?.into();
        let fd = reopen(&handle).map_err(|err| map_err(err.into()))?;
        let id = identity(&rustix::fs::fstat(&fd).map_err(|err| map_err(err.into()))?);
        let chosen = names.is_some();
        let names = match names {
            Some(names) => names
                .iter()
                .rev()
                .filter_map(|name| CString::new(name.as_bytes()).ok())
                .collect(),
            None => read_names(&fd).map_err(|err| map_err(err.into()))?,
        };
        let root_id = identity(&rustix::fs::fstat(&self.root).map_err(|err| map_err(err.into()))?);
        let staging = rustix::fs::statat(&self.root, STAGING_DIR, AtFlags::SYMLINK_NOFOLLOW)
            .ok()
            .filter(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory)
            .map(|stat| identity(&stat));
        Ok(TreeWalk {
            root_id,
            staging,
            frames: vec![Frame {
                fd: Some(fd),
                id,
                raw_name: CString::default(),
                prefix: String::new(),
                rel: Some(dir.clone()),
                names,
                used: HashSet::new(),
            }],
            chosen,
        })
    }
}

impl TreeWalk {
    /// The next entry, or `None` once the walk is over. `volume` must be the walked one.
    pub fn next(&mut self, volume: &Volume) -> Option<TreeItem> {
        loop {
            let depth = self.frames.len() - 1;
            let Some(raw) = self.frames[depth].names.pop() else {
                if depth == 0 {
                    return None;
                }
                self.frames.pop();
                continue;
            };
            match self.visit(volume, depth, raw) {
                Visit::Hidden => {}
                Visit::Item(item) => return Some(item),
                Visit::Folder(item, frame) => {
                    self.frames.push(frame);
                    return Some(item);
                }
            }
        }
    }

    /// A name for an entry of the walked folder that no entry has taken.
    pub fn unused_top_name(&mut self, wanted: &str) -> String {
        unique(&mut self.frames[0].used, wanted)
    }

    fn visit(&mut self, volume: &Volume, depth: usize, raw: CString) -> Visit {
        let frame = &self.frames[depth];
        if frame.id == self.root_id && raw.to_str().is_ok_and(is_staging_name) {
            return Visit::Hidden;
        }
        let rel = frame
            .rel
            .as_ref()
            .and_then(|rel| raw.to_str().ok().and_then(|name| rel.join(name).ok()));
        let prefix = frame.prefix.clone();
        let name = prefix + &unique(&mut self.frames[depth].used, &safe_name(&raw));
        let Ok(dir) = self.dir_fd(depth) else {
            return skip(name, SkipReason::Unreadable);
        };
        let stat = match rustix::fs::statat(&dir, raw.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Visit::Hidden,
            Err(_) => return skip(name, SkipReason::Unreadable),
        };
        if Some(identity(&stat)) == self.staging {
            return Visit::Hidden;
        }
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let opened = open_dir_at(&dir, &raw, identity(&stat));
                drop(dir);
                match opened {
                    Ok(fd) => Self::folder(depth, fd, raw, name, rel),
                    Err(_) => skip(name, SkipReason::Unreadable),
                }
            }
            FileType::RegularFile => match open_file_at(&dir, &raw) {
                Ok((file, stat)) => Visit::Item(TreeItem::File {
                    name,
                    file,
                    size: size_of(&stat),
                    modified: modified(&stat),
                }),
                Err(_) => skip(name, SkipReason::Unreadable),
            },
            FileType::Symlink => {
                drop(dir);
                self.link(volume, depth, raw, name, rel)
            }
            _ => skip(name, SkipReason::Special),
        }
    }

    /// Follows a symlink within the volume, through its volume path.
    fn link(
        &mut self,
        volume: &Volume,
        depth: usize,
        raw: CString,
        name: String,
        rel: Option<RelPath>,
    ) -> Visit {
        let Some(path) = rel else {
            return skip(name, SkipReason::LinkNotFollowed);
        };
        match volume.stat(&path).map(|meta| meta.kind) {
            Ok(EntryKind::File) => match volume.open_file(&path) {
                Ok((file, meta)) => Visit::Item(TreeItem::File {
                    name,
                    file,
                    size: meta.size.unwrap_or(0),
                    modified: meta.modified,
                }),
                Err(_) => skip(name, SkipReason::Unreadable),
            },
            Ok(EntryKind::Dir) if self.chosen && depth == 0 => {
                let opened = volume
                    .root
                    .open_dir(path.as_str())
                    .map_err(|_| Errno::ACCESS)
                    .and_then(|dir| reopen(&OwnedFd::from(dir)));
                match opened {
                    Ok(fd) => Self::folder(depth, fd, raw, name, Some(path)),
                    Err(_) => skip(name, SkipReason::Unreadable),
                }
            }
            Ok(EntryKind::Dir) => skip(name, SkipReason::LinkToFolder),
            Ok(EntryKind::Other) => skip(name, SkipReason::Special),
            Err(_) => skip(name, SkipReason::LinkNotFollowed),
        }
    }

    /// A folder opened as `fd`, entered next.
    fn folder(
        depth: usize,
        fd: OwnedFd,
        raw: CString,
        name: String,
        rel: Option<RelPath>,
    ) -> Visit {
        let read = rustix::fs::fstat(&fd).and_then(|stat| Ok((stat, read_names(&fd)?)));
        let Ok((stat, names)) = read else {
            return skip(name, SkipReason::Unreadable);
        };
        let frame = Frame {
            fd: (depth + 1 < KEEP_OPEN_DEPTH).then_some(fd),
            id: identity(&stat),
            raw_name: raw,
            prefix: format!("{name}/"),
            rel,
            names,
            used: HashSet::new(),
        };
        let item = TreeItem::Dir {
            name,
            modified: modified(&stat),
        };
        Visit::Folder(item, frame)
    }

    /// The folder at `depth`, reopened from the nearest one kept open if it isn't, and only if
    /// it is still the one that was listed.
    fn dir_fd(&self, depth: usize) -> rustix::io::Result<DirFd<'_>> {
        let kept = (0..=depth)
            .rev()
            .find(|&at| self.frames[at].fd.is_some())
            .unwrap_or(0);
        let Some(fd) = self.frames[kept].fd.as_ref() else {
            return Err(Errno::BADF);
        };
        if kept == depth {
            return Ok(DirFd::Kept(fd.as_fd()));
        }
        let mut reopened: Option<OwnedFd> = None;
        for frame in &self.frames[kept + 1..=depth] {
            let parent = reopened.as_ref().map_or(fd.as_fd(), AsFd::as_fd);
            reopened = Some(open_dir_at(&parent, &frame.raw_name, frame.id)?);
        }
        reopened.map(DirFd::Reopened).ok_or(Errno::BADF)
    }
}

fn skip(name: String, reason: SkipReason) -> Visit {
    Visit::Item(TreeItem::Skipped { name, reason })
}

enum DirFd<'a> {
    Kept(BorrowedFd<'a>),
    Reopened(OwnedFd),
}

impl AsFd for DirFd<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::Kept(fd) => *fd,
            Self::Reopened(fd) => fd.as_fd(),
        }
    }
}

/// cap-std opens folders with `O_PATH` on Linux, which cannot be listed.
fn reopen(fd: &impl AsFd) -> rustix::io::Result<OwnedFd> {
    rustix::fs::openat(fd, c".", dir_flags(), Mode::empty())
}

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// The folder `name` of `parent`, if it is still the one with identity `id`.
fn open_dir_at(parent: &impl AsFd, name: &CStr, id: (u64, u64)) -> rustix::io::Result<OwnedFd> {
    let fd = rustix::fs::openat(parent, name, dir_flags(), Mode::empty())?;
    if identity(&rustix::fs::fstat(&fd)?) == id {
        Ok(fd)
    } else {
        Err(Errno::NOENT)
    }
}

/// Non-blocking, so that a FIFO swapped in cannot hang the open.
fn open_file_at(parent: &impl AsFd, name: &CStr) -> rustix::io::Result<(File, Stat)> {
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::NOFOLLOW;
    let fd = rustix::fs::openat(parent, name, flags | OFlags::CLOEXEC, Mode::empty())?;
    let stat = rustix::fs::fstat(&fd)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Errno::NOENT);
    }
    Ok((File::from(fd), stat))
}

/// Names sorted bytewise, last first.
fn read_names(fd: &OwnedFd) -> rustix::io::Result<Vec<CString>> {
    let mut names = Vec::new();
    for entry in rustix::fs::Dir::read_from(fd)? {
        let name = entry?.file_name().to_owned();
        if !matches!(name.as_bytes(), b"." | b"..") {
            names.push(name);
        }
    }
    names.sort_unstable_by(|a, b| b.cmp(a));
    Ok(names)
}

fn safe_name(raw: &CStr) -> String {
    String::from_utf8_lossy(raw.to_bytes())
        .chars()
        .map(|c| if c.is_control() || c == '\\' { '_' } else { c })
        .collect()
}

fn unique(used: &mut HashSet<String>, name: &str) -> String {
    let mut candidate = name.to_owned();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = keep_both_name(name, n);
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

// The field types differ between platforms: what is a cast on one is a no-op on another.
#[allow(
    clippy::unnecessary_cast,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::useless_conversion
)]
fn identity(stat: &Stat) -> (u64, u64) {
    (stat.st_dev as u64, stat.st_ino as u64)
}

#[allow(clippy::unnecessary_cast, clippy::cast_sign_loss)]
fn size_of(stat: &Stat) -> u64 {
    stat.st_size as u64
}

#[allow(
    clippy::unnecessary_cast,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::useless_conversion
)]
fn modified(stat: &Stat) -> Option<SystemTime> {
    let secs = stat.st_mtime as i64;
    let nanos = Duration::from_nanos(stat.st_mtime_nsec as u64);
    if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64) + nanos)
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(secs.unsigned_abs()))?
            .checked_add(nanos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe_and_unique() {
        let mut used = HashSet::new();
        let raw = |bytes: &[u8]| CString::new(bytes).unwrap();
        assert_eq!(safe_name(&raw(b"a\nb\\c.txt")), "a_b_c.txt");
        assert_eq!(safe_name(&raw(b"caf\xe9.txt")), "caf\u{fffd}.txt");
        assert_eq!(unique(&mut used, "a.txt"), "a.txt");
        assert_eq!(unique(&mut used, "a.txt"), "a (2).txt");
        assert_eq!(unique(&mut used, "a.txt"), "a (3).txt");
    }
}
