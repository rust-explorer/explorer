use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
use rustix::io::Errno;

use super::super::path::STAGING_DIR;
use super::super::read::fs_path;
use super::hard_links::{HardLink, HardLinks};
use super::{USAGE_THREADS, USAGE_TOP_FILES, UsageNode};
use crate::storage::{RelPath, StorageError, TransferProgress, Volume};

/// Walks `path` with [`USAGE_THREADS`] workers and returns its folders with their own counts.
pub(super) fn scan(
    volume: &Volume,
    path: &RelPath,
    progress: &(dyn TransferProgress + Sync),
    max_dirs: usize,
) -> Result<Vec<UsageNode>, StorageError> {
    volume.refuse_reserved(path)?;
    let start: OwnedFd = volume
        .root
        .open_dir(fs_path(path))
        .map_err(|err| StorageError::from_io(path, err))?
        .into();
    let io_err = |err: Errno| StorageError::from_io(path, err.into());
    let start_stat = Measured::of(&rustix::fs::fstat(&start).map_err(io_err)?);
    let volume_root = Measured::of(&rustix::fs::fstat(&volume.root).map_err(io_err)?);
    let staging = rustix::fs::statat(&volume.root, STAGING_DIR, AtFlags::SYMLINK_NOFOLLOW)
        .ok()
        .map(|stat| Measured::of(&stat))
        .filter(|stat| stat.is_dir)
        .map(Measured::id);

    progress.add_bytes(start_stat.bytes);
    let scan = Scan {
        dev: start_stat.dev,
        root_id: volume_root.id(),
        staging,
        max_dirs,
        progress,
        state: Mutex::new(State {
            jobs: vec![Job {
                dir: Source::Opened(start),
                path: Vec::new(),
                node: 0,
                collapsed: false,
            }],
            busy: 0,
            cancelled: false,
            failed: false,
            hard_links: HardLinks::default(),
            nodes: vec![UsageNode {
                bytes: start_stat.bytes,
                ..UsageNode::default()
            }],
        }),
        wake: Condvar::new(),
    };
    let panicked = std::thread::scope(|threads| {
        let workers: Vec<_> = (0..USAGE_THREADS)
            .map(|_| threads.spawn(|| scan.work()))
            .collect();
        workers
            .into_iter()
            .map(std::thread::ScopedJoinHandle::join)
            .any(|joined| joined.is_err())
    });
    if panicked {
        return Err(StorageError::Io {
            path: path.clone(),
            source: io::Error::other("a usage worker panicked"),
        });
    }
    let mut state = scan
        .state
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    if state.cancelled {
        return Err(StorageError::Cancelled);
    }
    state.hard_links.settle(&mut state.nodes);
    Ok(state.nodes)
}

/// What a scan needs from a `stat`.
#[derive(Clone, Copy)]
struct Measured {
    dev: u64,
    ino: u64,
    nlink: u64,
    bytes: u64,
    is_dir: bool,
    is_file: bool,
}

impl Measured {
    // The field types differ between platforms: what is a cast on one is a no-op on another.
    #[allow(
        clippy::unnecessary_cast,
        clippy::cast_sign_loss,
        clippy::cast_lossless,
        clippy::useless_conversion
    )]
    fn of(stat: &Stat) -> Self {
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

    fn id(self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

/// A folder to read: the start folder, already open, or a sub-folder to open from its parent.
enum Source {
    Opened(OwnedFd),
    At {
        parent: Arc<OwnedFd>,
        name: CString,
        ino: u64,
    },
}

/// `path`: the folder's names from the scanned folder down. `collapsed`: the folder has no node
/// of its own and counts into `node`.
struct Job {
    dir: Source,
    path: Vec<CString>,
    node: usize,
    collapsed: bool,
}

struct State {
    jobs: Vec<Job>,
    busy: usize,
    cancelled: bool,
    /// A worker panicked: the others stop too.
    failed: bool,
    nodes: Vec<UsageNode>,
    hard_links: HardLinks,
}

struct Scan<'a> {
    dev: u64,
    root_id: (u64, u64),
    staging: Option<(u64, u64)>,
    max_dirs: usize,
    progress: &'a (dyn TransferProgress + Sync),
    state: Mutex<State>,
    wake: Condvar,
}

/// Ends a worker's job however the worker leaves it, panics included, so that the others never
/// wait for it forever.
struct Busy<'s, 'a> {
    scan: &'s Scan<'a>,
}

impl Drop for Busy<'_, '_> {
    fn drop(&mut self) {
        let mut state = self.scan.lock();
        state.busy -= 1;
        state.failed |= std::thread::panicking();
        drop(state);
        self.scan.wake.notify_all();
    }
}

/// What reading one folder found, merged into the tree afterwards under the lock.
#[derive(Default)]
struct Read {
    files: Vec<(String, u64)>,
    hard_links: Vec<(CString, Measured)>,
    subdirs: Vec<Subdir>,
    incomplete: bool,
}

struct Subdir {
    parent: Arc<OwnedFd>,
    name: CString,
    stat: Measured,
}

impl Scan<'_> {
    fn work(&self) {
        loop {
            let job = {
                let mut state = self.lock();
                loop {
                    if state.cancelled || state.failed {
                        return;
                    }
                    if let Some(job) = state.jobs.pop() {
                        state.busy += 1;
                        break job;
                    }
                    if state.busy == 0 {
                        self.wake.notify_all();
                        return;
                    }
                    state = self
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            let _busy = Busy { scan: self };
            if self.progress.is_cancelled() {
                self.lock().cancelled = true;
                return;
            }
            let read = self.read(&job.dir);
            let mut state = self.lock();
            self.merge(&mut state, &job, read);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self, source: &Source) -> Read {
        let mut read = Read::default();
        let Some(dir) = self.open(source) else {
            read.incomplete = true;
            self.progress.skip();
            return read;
        };
        let dir = Arc::new(dir);
        let dir_id = rustix::fs::fstat(&*dir).map(|stat| Measured::of(&stat).id());
        let entries = rustix::fs::Dir::read_from(&*dir);
        let (Ok(dir_id), Ok(entries)) = (dir_id, entries) else {
            read.incomplete = true;
            self.progress.skip();
            return read;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                read.incomplete = true;
                self.progress.skip();
                break;
            };
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            match rustix::fs::statat(&*dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => self.add_entry(&mut read, &dir, dir_id, name, Measured::of(&stat)),
                Err(Errno::NOENT) => {}
                Err(_) => {
                    read.incomplete = true;
                    self.progress.skip();
                }
            }
        }
        read
    }

    /// Opens a sub-folder without following symlinks, and only if it is still the folder that
    /// was measured.
    fn open(&self, source: &Source) -> Option<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let (parent, name, ino) = match source {
            // cap-std opens folders with O_PATH on Linux, which cannot be listed: reopen it.
            Source::Opened(fd) => return rustix::fs::openat(fd, c".", flags, Mode::empty()).ok(),
            Source::At { parent, name, ino } => (parent, name, *ino),
        };
        let fd = rustix::fs::openat(parent.as_fd(), name.as_c_str(), flags, Mode::empty()).ok()?;
        let stat = Measured::of(&rustix::fs::fstat(&fd).ok()?);
        (stat.id() == (self.dev, ino)).then_some(fd)
    }

    fn add_entry(
        &self,
        read: &mut Read,
        dir: &Arc<OwnedFd>,
        dir_id: (u64, u64),
        name: &CStr,
        stat: Measured,
    ) {
        if (dir_id == self.root_id && name.to_bytes() == STAGING_DIR.as_bytes())
            || Some(stat.id()) == self.staging
        {
            return;
        }
        if stat.is_dir {
            if stat.dev == self.dev {
                read.subdirs.push(Subdir {
                    parent: Arc::clone(dir),
                    name: name.to_owned(),
                    stat,
                });
            }
            return;
        }
        if stat.is_file && stat.nlink > 1 {
            read.hard_links.push((name.to_owned(), stat));
            return;
        }
        self.progress.add_file();
        self.progress.add_bytes(stat.bytes);
        read.files.push((lossy(name), stat.bytes));
    }

    fn merge(&self, state: &mut State, job: &Job, read: Read) {
        let target = job.node;
        let node = &mut state.nodes[target];
        node.incomplete |= read.incomplete;
        node.files += read.files.len() as u64;
        node.bytes += read.files.iter().map(|(_, bytes)| bytes).sum::<u64>();
        if !job.collapsed {
            node.top_files.extend(read.files);
            keep_biggest(node);
        }
        for (name, stat) in read.hard_links {
            let mut path = job.path.clone();
            path.push(name);
            let link = HardLink {
                name: lossy(path.last().expect("just pushed")),
                path,
                node: target,
                collapsed: job.collapsed,
                bytes: stat.bytes,
            };
            if state.hard_links.offer(stat.id(), link) {
                self.progress.add_file();
                self.progress.add_bytes(stat.bytes);
            }
        }
        for subdir in read.subdirs {
            self.progress.add_bytes(subdir.stat.bytes);
            let name = lossy(&subdir.name);
            let mut path = job.path.clone();
            path.push(subdir.name.clone());
            let dir = Source::At {
                parent: subdir.parent,
                name: subdir.name,
                ino: subdir.stat.ino,
            };
            let job = if state.nodes.len() < self.max_dirs {
                let index = state.nodes.len();
                state.nodes.push(UsageNode {
                    name,
                    parent: target,
                    bytes: subdir.stat.bytes,
                    ..UsageNode::default()
                });
                state.nodes[target].children.push(index);
                Job {
                    dir,
                    path,
                    node: index,
                    collapsed: false,
                }
            } else {
                let node = &mut state.nodes[target];
                node.bytes += subdir.stat.bytes;
                node.dirs += 1;
                node.collapsed = true;
                Job {
                    dir,
                    path,
                    node: target,
                    collapsed: true,
                }
            };
            state.jobs.push(job);
        }
    }
}

/// Keeps the [`USAGE_TOP_FILES`] biggest files by name and folds the others into a count.
pub(super) fn keep_biggest(node: &mut UsageNode) {
    if node.top_files.len() <= USAGE_TOP_FILES {
        return;
    }
    node.top_files
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (_, bytes) in node.top_files.drain(USAGE_TOP_FILES..) {
        node.other_files += 1;
        node.other_bytes += bytes;
    }
}

pub(super) fn lossy(name: &CStr) -> String {
    String::from_utf8_lossy(name.to_bytes()).into_owned()
}
