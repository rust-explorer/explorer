use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use shared::api::error::ErrorBody;
use shared::api::tasks::{TaskInfo, TaskSpec, TaskState};
use tokio::sync::Semaphore;

use super::{FINISHED_TTL, MAX_ACTIVE_PER_USER, MAX_RUNNING, MAX_USAGE_TREES};
use crate::storage::{TransferPlan, UsageTree};

const ID_LEN: usize = 32;

/// What a running task reports as it goes, read by every poll.
#[derive(Default)]
pub struct Progress {
    cancelled: AtomicBool,
    files_done: AtomicU64,
    bytes_done: AtomicU64,
    skipped: AtomicU64,
    files_total: OnceLock<u64>,
    bytes_total: OnceLock<u64>,
    final_path: OnceLock<String>,
    usage: Mutex<Option<UsageTree>>,
}

impl Progress {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn add_bytes(&self, n: u64) {
        self.bytes_done.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_file(&self) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Only the first call counts, as for the other totals and the final path.
    pub fn set_files_total(&self, n: u64) {
        let _ = self.files_total.set(n);
    }

    pub fn set_bytes_total(&self, n: u64) {
        let _ = self.bytes_total.set(n);
    }

    /// The totals of a copy or move, from its plan.
    pub fn set_plan(&self, plan: &TransferPlan) {
        self.set_files_total(plan.files);
        self.set_bytes_total(plan.bytes);
    }

    pub fn set_final_path(&self, path: String) {
        let _ = self.final_path.set(path);
    }

    /// A usage scan's result, which the task keeps once done.
    pub fn set_usage(&self, tree: UsageTree) {
        *self.usage.lock().unwrap_or_else(PoisonError::into_inner) = Some(tree);
    }

    /// Files and bytes done, and entries skipped, so far.
    pub fn counts(&self) -> (u64, u64, u64) {
        (
            self.files_done.load(Ordering::Relaxed),
            self.bytes_done.load(Ordering::Relaxed),
            self.skipped.load(Ordering::Relaxed),
        )
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl crate::storage::TransferProgress for Progress {
    fn is_cancelled(&self) -> bool {
        Progress::is_cancelled(self)
    }

    fn add_bytes(&self, n: u64) {
        Progress::add_bytes(self, n);
    }

    fn add_file(&self) {
        Progress::add_file(self);
    }

    fn skip(&self) {
        Progress::skip(self);
    }

    fn planned(&self, plan: &TransferPlan) {
        self.set_plan(plan);
    }
}

/// How a task ended.
pub(super) enum Outcome {
    Done,
    Failed(ErrorBody),
    Cancelled,
}

struct Entry {
    seq: u64,
    user: String,
    spec: TaskSpec,
    state: TaskState,
    progress: Arc<Progress>,
    error: Option<ErrorBody>,
    finished_at: Option<Instant>,
    usage: Option<Arc<UsageTree>>,
}

impl Entry {
    fn info(&self, id: &str) -> TaskInfo {
        let progress = &self.progress;
        TaskInfo {
            id: id.to_owned(),
            spec: self.spec.clone(),
            state: self.state,
            files_done: progress.files_done.load(Ordering::Relaxed),
            files_total: progress.files_total.get().copied(),
            bytes_done: progress.bytes_done.load(Ordering::Relaxed),
            bytes_total: progress.bytes_total.get().copied(),
            skipped: progress.skipped.load(Ordering::Relaxed),
            final_path: progress.final_path.get().cloned(),
            error: self.error.clone(),
        }
    }

    fn finish(&mut self, state: TaskState, now: Instant) {
        self.state = state;
        self.finished_at = Some(now);
    }
}

#[derive(Default)]
struct Entries {
    by_id: HashMap<String, Entry>,
    next_seq: u64,
}

impl Entries {
    /// After task `id` stored a usage tree: drops its user's other trees of the same folder,
    /// then the oldest others beyond [`MAX_USAGE_TREES`] in all.
    fn keep_usage(&mut self, id: &str) {
        let Some(kept) = self.by_id.get(id) else {
            return;
        };
        let (user, source) = (kept.user.clone(), kept.spec.source().clone());
        let mut trees: Vec<(u64, &mut Entry)> = self
            .by_id
            .iter_mut()
            .filter(|(_, entry)| entry.user == user && entry.usage.is_some())
            .filter(|(other, _)| other.as_str() != id)
            .filter_map(|(_, entry)| {
                if entry.spec.source() == &source {
                    entry.usage = None;
                    None
                } else {
                    Some((entry.seq, entry))
                }
            })
            .collect();
        trees.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));
        for (_, entry) in trees.into_iter().skip(MAX_USAGE_TREES - 1) {
            entry.usage = None;
        }
    }

    /// Forgets tasks finished more than [`FINISHED_TTL`] before `now`.
    fn prune(&mut self, now: Instant) {
        self.by_id.retain(|_, entry| {
            entry
                .finished_at
                .is_none_or(|at| now.saturating_duration_since(at) <= FINISHED_TTL)
        });
    }
}

/// What [`Tasks::cancel_or_dismiss`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    Dismissed,
}

/// The background tasks of every user, in memory: a restart loses them.
pub struct Tasks {
    entries: Mutex<Entries>,
    pub(super) permits: Arc<Semaphore>,
}

impl Default for Tasks {
    fn default() -> Self {
        Self::new()
    }
}

impl Tasks {
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(MAX_RUNNING)
    }

    /// Runs at most `limit` tasks at once; with 0, every task stays queued.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            entries: Mutex::default(),
            permits: Arc::new(Semaphore::new(limit)),
        }
    }

    /// The tasks of `user` in creation order.
    pub fn list(&self, user: &str) -> Vec<TaskInfo> {
        self.list_at(user, Instant::now())
    }

    /// Like [`Tasks::list`] at `now`, first forgetting tasks finished more than
    /// [`FINISHED_TTL`] before.
    fn list_at(&self, user: &str, now: Instant) -> Vec<TaskInfo> {
        let mut entries = self.lock();
        entries.prune(now);
        let mut mine: Vec<(&String, &Entry)> = entries
            .by_id
            .iter()
            .filter(|(_, entry)| entry.user == user)
            .collect();
        mine.sort_by_key(|(_, entry)| entry.seq);
        mine.into_iter().map(|(id, entry)| entry.info(id)).collect()
    }

    /// The volume and tree of a finished usage scan of `user`.
    pub fn usage(&self, user: &str, id: &str) -> Option<(String, Arc<UsageTree>)> {
        let mut entries = self.lock();
        entries.prune(Instant::now());
        let entry = entries
            .by_id
            .get(id)
            .filter(|entry| entry.user == user && entry.state == TaskState::Done)?;
        let TaskSpec::Usage { source } = &entry.spec else {
            return None;
        };
        let tree = entry.usage.as_ref()?;
        Some((source.volume.clone(), Arc::clone(tree)))
    }

    /// Cancels a queued or running task, or forgets a finished one; `None` when `id` is unknown
    /// or another user's. A queued task is cancelled at once, a running one when its runner
    /// notices.
    pub fn cancel_or_dismiss(&self, user: &str, id: &str) -> Option<Stopped> {
        let mut entries = self.lock();
        let entry = entries
            .by_id
            .get_mut(id)
            .filter(|entry| entry.user == user)?;
        match entry.state {
            TaskState::Queued => {
                entry.progress.cancel();
                entry.finish(TaskState::Cancelled, Instant::now());
            }
            TaskState::Running => entry.progress.cancel(),
            TaskState::Done | TaskState::Failed | TaskState::Cancelled => {
                entries.by_id.remove(id);
                return Some(Stopped::Dismissed);
            }
        }
        Some(Stopped::Cancelled)
    }

    /// Registers a queued task for `user`; `None` when they already have
    /// [`MAX_ACTIVE_PER_USER`] tasks queued or running.
    pub(super) fn insert(&self, user: &str, spec: TaskSpec) -> Option<(String, TaskInfo)> {
        let mut entries = self.lock();
        let active = entries
            .by_id
            .values()
            .filter(|entry| entry.user == user && entry.state.is_active())
            .count();
        if active >= MAX_ACTIVE_PER_USER {
            return None;
        }
        let id = random_id();
        let seq = entries.next_seq;
        entries.next_seq += 1;
        let entry = Entry {
            seq,
            user: user.to_owned(),
            spec,
            state: TaskState::Queued,
            progress: Arc::default(),
            error: None,
            finished_at: None,
            usage: None,
        };
        let info = entry.info(&id);
        entries.by_id.insert(id.clone(), entry);
        Some((id, info))
    }

    /// Marks a queued task running; `None` when it was cancelled meanwhile.
    pub(super) fn start(&self, id: &str) -> Option<Arc<Progress>> {
        let mut entries = self.lock();
        let entry = entries
            .by_id
            .get_mut(id)
            .filter(|entry| entry.state == TaskState::Queued)?;
        entry.state = TaskState::Running;
        Some(Arc::clone(&entry.progress))
    }

    pub(super) fn finish(&self, id: &str, outcome: Outcome) {
        let mut entries = self.lock();
        let Some(entry) = entries.by_id.get_mut(id) else {
            return;
        };
        let tree = entry
            .progress
            .usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let state = match outcome {
            Outcome::Done => TaskState::Done,
            Outcome::Cancelled => TaskState::Cancelled,
            Outcome::Failed(error) => {
                entry.error = Some(error);
                TaskState::Failed
            }
        };
        entry.finish(state, Instant::now());
        if let Some(tree) = tree.filter(|_| state == TaskState::Done) {
            entry.usage = Some(Arc::new(tree));
            entries.keep_usage(id);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 128 random bits as lowercase hex.
fn random_id() -> String {
    let bytes: [u8; ID_LEN / 2] = rand::random();
    let mut id = String::with_capacity(ID_LEN);
    for byte in bytes {
        write!(id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    id
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use shared::api::tasks::Location;

    use super::*;

    fn spec(path: &str) -> TaskSpec {
        TaskSpec::Delete {
            source: Location {
                volume: "media".to_owned(),
                path: path.to_owned(),
            },
        }
    }

    fn states(tasks: &Tasks, user: &str) -> Vec<TaskState> {
        tasks.list(user).iter().map(|task| task.state).collect()
    }

    #[test]
    fn lists_a_users_tasks_in_creation_order() {
        let tasks = Tasks::new();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|path| tasks.insert("ada", spec(path)).unwrap().0)
            .collect();
        tasks.insert("bob", spec("d")).unwrap();

        let listed: Vec<String> = tasks.list("ada").into_iter().map(|task| task.id).collect();
        assert_eq!(listed, ids);
        assert_eq!(tasks.list("bob").len(), 1);
        assert!(tasks.list("eve").is_empty());
        assert!(
            ids.iter().all(|id| id.len() == ID_LEN
                && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        );
    }

    #[test]
    fn caps_the_active_tasks_of_a_user() {
        let tasks = Tasks::new();
        let ids: Vec<String> = (0..MAX_ACTIVE_PER_USER)
            .map(|_| tasks.insert("ada", spec("a")).unwrap().0)
            .collect();
        assert!(tasks.insert("ada", spec("a")).is_none());
        assert!(tasks.insert("bob", spec("a")).is_some());

        tasks.start(&ids[0]).unwrap();
        tasks.finish(&ids[0], Outcome::Done);
        assert!(tasks.insert("ada", spec("a")).is_some());
        assert!(tasks.insert("ada", spec("a")).is_none());
    }

    #[test]
    fn prunes_finished_tasks_after_the_ttl() {
        let tasks = Tasks::new();
        let (done, _) = tasks.insert("ada", spec("a")).unwrap();
        let (queued, _) = tasks.insert("ada", spec("b")).unwrap();
        tasks.start(&done).unwrap();
        tasks.finish(&done, Outcome::Done);

        let soon = Instant::now() + FINISHED_TTL / 2;
        assert_eq!(tasks.list_at("ada", soon).len(), 2);

        let later = Instant::now() + FINISHED_TTL + Duration::from_secs(1);
        let left: Vec<String> = tasks
            .list_at("ada", later)
            .into_iter()
            .map(|task| task.id)
            .collect();
        assert_eq!(left, [queued]);
    }

    #[test]
    fn cancel_or_dismiss_by_state() {
        let tasks = Tasks::new();
        let (queued, _) = tasks.insert("ada", spec("a")).unwrap();
        let (running, _) = tasks.insert("ada", spec("b")).unwrap();
        let progress = tasks.start(&running).unwrap();

        assert_eq!(tasks.cancel_or_dismiss("bob", &queued), None);
        assert_eq!(tasks.cancel_or_dismiss("ada", "unknown"), None);

        let cancelled = Some(Stopped::Cancelled);
        assert_eq!(tasks.cancel_or_dismiss("ada", &queued), cancelled);
        assert_eq!(tasks.cancel_or_dismiss("ada", &running), cancelled);
        assert!(progress.is_cancelled());
        assert_eq!(
            states(&tasks, "ada"),
            [TaskState::Cancelled, TaskState::Running]
        );
        assert!(tasks.start(&queued).is_none());

        tasks.finish(&running, Outcome::Cancelled);
        let dismissed = Some(Stopped::Dismissed);
        assert_eq!(tasks.cancel_or_dismiss("ada", &queued), dismissed);
        assert_eq!(tasks.cancel_or_dismiss("ada", &running), dismissed);
        assert!(tasks.list("ada").is_empty());
        assert_eq!(tasks.cancel_or_dismiss("ada", &running), None);
    }

    #[test]
    fn progress_shows_in_the_info() {
        let tasks = Tasks::new();
        let (id, info) = tasks.insert("ada", spec("a")).unwrap();
        assert_eq!((info.files_total, info.bytes_total), (None, None));
        let progress = tasks.start(&id).unwrap();
        progress.set_files_total(3);
        progress.set_files_total(9);
        progress.set_bytes_total(100);
        progress.add_file();
        progress.add_bytes(40);
        progress.skip();
        progress.set_final_path("b/a".to_owned());

        let info = &tasks.list("ada")[0];
        assert_eq!(info.state, TaskState::Running);
        assert_eq!((info.files_done, info.files_total), (1, Some(3)));
        assert_eq!((info.bytes_done, info.bytes_total), (40, Some(100)));
        assert_eq!(info.skipped, 1);
        assert_eq!(info.final_path.as_deref(), Some("b/a"));
    }

    fn usage_spec(path: &str) -> TaskSpec {
        TaskSpec::Usage {
            source: Location {
                volume: "media".to_owned(),
                path: path.to_owned(),
            },
        }
    }

    /// Runs a usage task of `user` on `path` to `outcome`, with a tree of an empty folder.
    fn scanned(tasks: &Tasks, user: &str, spec: TaskSpec, outcome: Outcome) -> String {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(data.path().join("media")).unwrap();
        let volumes = crate::storage::Volumes::discover(data.path()).unwrap();
        let media = volumes.get("media").unwrap();
        let tree = crate::storage::usage(
            media,
            &crate::storage::RelPath::root(),
            &Progress::default(),
        )
        .unwrap();
        let (id, _) = tasks.insert(user, spec).unwrap();
        let progress = tasks.start(&id).unwrap();
        progress.set_usage(tree);
        assert!(tasks.usage(user, &id).is_none());
        tasks.finish(&id, outcome);
        id
    }

    #[test]
    fn usage_is_kept_for_its_done_task_only() {
        let tasks = Tasks::new();
        let id = scanned(&tasks, "ada", usage_spec(""), Outcome::Done);
        let (volume, tree) = tasks.usage("ada", &id).unwrap();
        assert_eq!(volume, "media");
        assert!(tree.root().is_root());
        assert!(tasks.usage("bob", &id).is_none());

        let delete = scanned(&tasks, "ada", spec("a"), Outcome::Done);
        assert!(tasks.usage("ada", &delete).is_none());
        let cancelled = scanned(&tasks, "ada", usage_spec("b"), Outcome::Cancelled);
        assert!(tasks.usage("ada", &cancelled).is_none());
    }

    #[test]
    fn usage_keeps_one_tree_per_folder() {
        let tasks = Tasks::new();
        let first = scanned(&tasks, "ada", usage_spec("Films"), Outcome::Done);
        let other_folder = scanned(&tasks, "ada", usage_spec("Music"), Outcome::Done);
        let other_user = scanned(&tasks, "bob", usage_spec("Films"), Outcome::Done);
        let second = scanned(&tasks, "ada", usage_spec("Films"), Outcome::Done);

        assert!(tasks.usage("ada", &first).is_none());
        assert!(tasks.usage("ada", &second).is_some());
        assert!(tasks.usage("ada", &other_folder).is_some());
        assert!(tasks.usage("bob", &other_user).is_some());
        assert_eq!(tasks.list("ada").len(), 3);
    }

    #[test]
    fn usage_keeps_the_newest_trees_of_a_user() {
        let tasks = Tasks::new();
        let ids: Vec<String> = (0..=MAX_USAGE_TREES)
            .map(|i| scanned(&tasks, "ada", usage_spec(&format!("f{i}")), Outcome::Done))
            .collect();
        let bob = scanned(&tasks, "bob", usage_spec("f0"), Outcome::Done);

        assert!(tasks.usage("ada", &ids[0]).is_none());
        assert!(ids[1..].iter().all(|id| tasks.usage("ada", id).is_some()));
        assert!(tasks.usage("bob", &bob).is_some());
    }

    #[test]
    fn failure_keeps_its_error() {
        let tasks = Tasks::new();
        let (id, _) = tasks.insert("ada", spec("a")).unwrap();
        tasks.start(&id).unwrap();
        let error = ErrorBody {
            code: "permission_denied".to_owned(),
            message: "Permission denied on media/a".to_owned(),
            details: None,
            request_id: Some("req".to_owned()),
        };
        tasks.finish(&id, Outcome::Failed(error.clone()));

        let info = &tasks.list("ada")[0];
        assert_eq!(info.state, TaskState::Failed);
        assert_eq!(info.error, Some(error));
    }
}
