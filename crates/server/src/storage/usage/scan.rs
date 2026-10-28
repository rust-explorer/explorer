use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rustix::fs::AtFlags;
use rustix::io::Errno;

use super::super::path::STAGING_DIR;
use super::super::read::fs_path;
use super::folder::{Measured, Source};
use super::hard_links::{HardLink, HardLinks, Offered};
use super::{USAGE_THREADS, USAGE_TOP_FILES, UsageLimits, UsageNode};
use crate::storage::{RelPath, StorageError, TransferProgress, Volume};

/// Walks `path` with [`USAGE_THREADS`] workers and returns its folders with their own counts.
pub(super) fn scan(
    volume: &Volume,
    path: &RelPath,
    progress: &(dyn TransferProgress + Sync),
    limits: UsageLimits,
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
        start,
        dev: start_stat.dev,
        root_id: volume_root.id(),
        staging,
        limits,
        progress,
        state: Mutex::new(State {
            jobs: vec![Job {
                dir: Source::Start,
                path: Vec::new(),
                node: 0,
                collapsed: false,
                unnamed: false,
            }],
            busy: 0,
            cancelled: false,
            failed: false,
            hard_links: HardLinks::default(),
            named_files: 0,
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

/// `path`: the folder's names from the scanned folder down. `collapsed`: the folder has no node
/// of its own and counts into `node`. `unnamed`: its name, or a parent's, is not UTF-8, so it
/// counts into `node` among the other files.
struct Job {
    dir: Source,
    path: Vec<CString>,
    node: usize,
    collapsed: bool,
    unnamed: bool,
}

struct State {
    jobs: Vec<Job>,
    busy: usize,
    cancelled: bool,
    /// A worker panicked: the others stop too.
    failed: bool,
    nodes: Vec<UsageNode>,
    hard_links: HardLinks,
    /// File names kept in all the nodes' `top_files`.
    named_files: usize,
}

struct Scan<'a> {
    start: OwnedFd,
    dev: u64,
    root_id: (u64, u64),
    staging: Option<(u64, u64)>,
    limits: UsageLimits,
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

/// What reading one folder found, merged into the tree afterwards under the lock. `unnamed`:
/// the count and bytes of the files whose name is not UTF-8.
#[derive(Default)]
struct Read {
    files: Vec<(String, u64)>,
    unnamed: (u64, u64),
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
            let read = self.read(&job);
            let mut state = self.lock();
            self.merge(&mut state, &job, read);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read(&self, job: &Job) -> Read {
        let mut read = Read::default();
        let Some(dir) = job.dir.open(&self.start, &job.path, self.dev) else {
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
        if let Ok(name) = name.to_str() {
            read.files.push((name.to_owned(), stat.bytes));
        } else {
            read.unnamed.0 += 1;
            read.unnamed.1 += stat.bytes;
        }
    }

    fn merge(&self, state: &mut State, job: &Job, read: Read) {
        let target = job.node;
        let names_left = state.named_files < self.limits.max_named_files;
        let node = &mut state.nodes[target];
        node.incomplete |= read.incomplete;
        let (unnamed_files, unnamed_bytes) = read.unnamed;
        node.files += read.files.len() as u64 + unnamed_files;
        let named_bytes = read.files.iter().map(|(_, bytes)| bytes).sum::<u64>();
        node.bytes += named_bytes + unnamed_bytes;
        if job.unnamed && !job.collapsed {
            node.other_files += read.files.len() as u64 + unnamed_files;
            node.other_bytes += named_bytes + unnamed_bytes;
        } else if !job.collapsed && !names_left {
            node.other_files += read.files.len() as u64 + unnamed_files;
            node.other_bytes += named_bytes + unnamed_bytes;
        } else if !job.collapsed {
            node.other_files += unnamed_files;
            node.other_bytes += unnamed_bytes;
            let kept = node.top_files.len();
            node.top_files.extend(read.files);
            keep_biggest(node);
            let added = node.top_files.len() - kept;
            state.named_files += added;
        }
        for (name, stat) in read.hard_links {
            let mut path = job.path.clone();
            path.push(name);
            let name = path
                .last()
                .and_then(|name| name.to_str().ok())
                .filter(|_| !job.unnamed)
                .map(str::to_owned);
            let link = HardLink {
                name,
                path,
                node: target,
                collapsed: job.collapsed,
                bytes: stat.bytes,
            };
            match state.hard_links.offer(stat.id(), link) {
                Offered::New => {}
                Offered::Seen => continue,
                // Too many to track: counted where seen, maybe more than once.
                Offered::Full(link) => link.credit(&mut state.nodes),
            }
            self.progress.add_file();
            self.progress.add_bytes(stat.bytes);
        }
        for subdir in read.subdirs {
            self.progress.add_bytes(subdir.stat.bytes);
            let name = subdir.name.to_str().ok().map(str::to_owned);
            let mut path = job.path.clone();
            path.push(subdir.name.clone());
            let dir = Source::sub(subdir.parent, subdir.name, subdir.stat.ino, path.len());
            let unnamed = job.unnamed || name.is_none();
            let job = match name {
                Some(name)
                    if !job.unnamed && !job.collapsed && state.nodes.len() < self.limits.max_dirs =>
                {
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
                        unnamed: false,
                    }
                }
                _ => {
                    let collapsed = job.collapsed || !unnamed;
                    let node = &mut state.nodes[target];
                    node.bytes += subdir.stat.bytes;
                    node.dirs += 1;
                    if collapsed {
                        node.collapsed = true;
                    } else {
                        node.other_bytes += subdir.stat.bytes;
                    }
                    Job {
                        dir,
                        path,
                        node: target,
                        collapsed,
                        unnamed,
                    }
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
