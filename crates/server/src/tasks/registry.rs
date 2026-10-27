use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use shared::api::error::ErrorBody;
use shared::api::tasks::{TaskInfo, TaskSpec, TaskState};
use tokio::sync::Semaphore;

use super::{FINISHED_TTL, MAX_RUNNING};

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

    pub fn set_final_path(&self, path: String) {
        let _ = self.final_path.set(path);
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
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
        entries.by_id.retain(|_, entry| {
            entry
                .finished_at
                .is_none_or(|at| now.saturating_duration_since(at) <= FINISHED_TTL)
        });
        let mut mine: Vec<(&String, &Entry)> = entries
            .by_id
            .iter()
            .filter(|(_, entry)| entry.user == user)
            .collect();
        mine.sort_by_key(|(_, entry)| entry.seq);
        mine.into_iter().map(|(id, entry)| entry.info(id)).collect()
    }

    /// Cancels a queued or running task, or forgets a finished one; false when `id` is unknown
    /// or another user's. A queued task is cancelled at once, a running one when its runner
    /// notices.
    pub fn cancel_or_dismiss(&self, user: &str, id: &str) -> bool {
        let mut entries = self.lock();
        let Some(entry) = entries.by_id.get_mut(id).filter(|entry| entry.user == user) else {
            return false;
        };
        match entry.state {
            TaskState::Queued => {
                entry.progress.cancel();
                entry.finish(TaskState::Cancelled, Instant::now());
            }
            TaskState::Running => entry.progress.cancel(),
            TaskState::Done | TaskState::Failed | TaskState::Cancelled => {
                entries.by_id.remove(id);
            }
        }
        true
    }

    /// Registers a queued task for `user`.
    pub(super) fn insert(&self, user: &str, spec: TaskSpec) -> (String, TaskInfo) {
        let mut entries = self.lock();
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
        };
        let info = entry.info(&id);
        entries.by_id.insert(id.clone(), entry);
        (id, info)
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
        let state = match outcome {
            Outcome::Done => TaskState::Done,
            Outcome::Cancelled => TaskState::Cancelled,
            Outcome::Failed(error) => {
                entry.error = Some(error);
                TaskState::Failed
            }
        };
        entry.finish(state, Instant::now());
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
            .map(|path| tasks.insert("ada", spec(path)).0)
            .collect();
        tasks.insert("bob", spec("d"));

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
    fn prunes_finished_tasks_after_the_ttl() {
        let tasks = Tasks::new();
        let (done, _) = tasks.insert("ada", spec("a"));
        let (queued, _) = tasks.insert("ada", spec("b"));
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
        let (queued, _) = tasks.insert("ada", spec("a"));
        let (running, _) = tasks.insert("ada", spec("b"));
        let progress = tasks.start(&running).unwrap();

        assert!(!tasks.cancel_or_dismiss("bob", &queued));
        assert!(!tasks.cancel_or_dismiss("ada", "unknown"));

        assert!(tasks.cancel_or_dismiss("ada", &queued));
        assert!(tasks.cancel_or_dismiss("ada", &running));
        assert!(progress.is_cancelled());
        assert_eq!(
            states(&tasks, "ada"),
            [TaskState::Cancelled, TaskState::Running]
        );
        assert!(tasks.start(&queued).is_none());

        tasks.finish(&running, Outcome::Cancelled);
        assert!(tasks.cancel_or_dismiss("ada", &queued));
        assert!(tasks.cancel_or_dismiss("ada", &running));
        assert!(tasks.list("ada").is_empty());
        assert!(!tasks.cancel_or_dismiss("ada", &running));
    }

    #[test]
    fn progress_shows_in_the_info() {
        let tasks = Tasks::new();
        let (id, info) = tasks.insert("ada", spec("a"));
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

    #[test]
    fn failure_keeps_its_error() {
        let tasks = Tasks::new();
        let (id, _) = tasks.insert("ada", spec("a"));
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
