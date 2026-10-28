mod decisions;
mod lifecycle;
mod polling;

use std::collections::HashMap;

use shared::api::browse::EntryKind;
use shared::api::tasks::{Location, TaskInfo, TaskSpec, TaskState};

use crate::api::ApiFailure;
use crate::conflict::Decision;
use crate::tasks::conflict::Conflict;

pub const POLL_EVERY_MS: u32 = 1000;
const TASK_NOT_FOUND: &str = "task_not_found";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskKey(pub u64);

/// A task this tab started, or adopted from the server after a reload.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub key: TaskKey,
    pub spec: TaskSpec,
    /// The server's view, once it created the task.
    pub info: Option<TaskInfo>,
    /// The server refused the task, or no longer knows it.
    lost: Option<ApiFailure>,
    cancel_requested: bool,
    /// The tasks of one user action share a batch, for "apply to all".
    pub batch: u64,
    /// What the source is, when the caller knows: only a file may replace a file.
    source_kind: Option<EntryKind>,
    /// The destination exists: the task waits for the user's decision.
    conflict: Option<Conflict>,
    /// Its page opens at this folder once the server gives it an id.
    pub(super) open_at: Option<String>,
    /// When this tab saw it finish.
    finished_at_ms: Option<i64>,
    /// It had finished before this tab took it over: when is unknown.
    finished_before: bool,
    /// Cleared from the activity panel, but still known for its result.
    dismissed: bool,
    /// Why the last cancel request failed.
    cancel_failure: Option<ApiFailure>,
}

impl Task {
    pub fn conflict(&self) -> Option<&Conflict> {
        self.conflict.as_ref()
    }

    /// Whether the conflict prompt may offer Replace.
    pub fn allows_replace(&self) -> bool {
        self.source_kind == Some(EntryKind::File)
            && self
                .conflict
                .as_ref()
                .is_some_and(|conflict| conflict.kind == EntryKind::File)
    }

    pub fn state(&self) -> TaskState {
        match (&self.lost, &self.info) {
            (Some(_), _) => TaskState::Failed,
            (None, Some(info)) => info.state,
            (None, None) => TaskState::Queued,
        }
    }

    pub fn is_active(&self) -> bool {
        self.state().is_active()
    }

    pub fn failure(&self) -> Option<ApiFailure> {
        self.lost.clone().or_else(|| {
            let error = self.info.as_ref()?.error.clone()?;
            Some(ApiFailure::Server(error))
        })
    }

    pub(in crate::tasks) fn dismissed(&self) -> bool {
        self.dismissed
    }

    pub fn cancel_failure(&self) -> Option<&ApiFailure> {
        self.cancel_failure.as_ref()
    }

    pub fn finished_at_ms(&self) -> Option<i64> {
        self.finished_at_ms
    }

    /// The server's id, while the server has the task.
    pub fn server_id(&self) -> Option<&str> {
        self.info
            .as_ref()
            .filter(|_| self.lost.is_none())
            .map(|info| info.id.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Creates the task on the server.
    Post { key: TaskKey, spec: TaskSpec },
    /// Cancels the task, or dismisses it once finished.
    Delete { id: String },
    /// Lists the tasks after `ms`.
    PollAfter { ms: u32 },
    /// Something changed on disk: listings reload.
    Refresh,
    /// Shows the result page of the usage task `id`, at the folder `at`.
    Open { id: String, at: Location },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Posted {
        key: TaskKey,
        info: Box<TaskInfo>,
    },
    PostFailed {
        key: TaskKey,
        failure: ApiFailure,
    },
    Polled(Vec<TaskInfo>),
    PollFailed(ApiFailure),
    /// The `Delete` of task `id` failed.
    DeleteFailed {
        id: String,
        failure: ApiFailure,
    },
}

/// The tab's tasks: events in, requests out. Polls only while one of them is queued or running.
#[derive(Debug, Default)]
pub struct TaskList {
    tasks: Vec<Task>,
    next_key: u64,
    polling: bool,
    /// The server ids known when the pending poll was issued; a task posted since then may be
    /// missing from its answer.
    polled: Vec<String>,
    next_batch: u64,
    /// "Apply to all" answers, by batch.
    decisions: HashMap<u64, Decision>,
    /// A poll found the session gone: polling stops until the user signs in again.
    signed_out: bool,
    /// The server's tasks were taken over, or could not be listed.
    adopted: bool,
}

impl TaskList {
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The tasks the activity panel lists.
    pub fn shown(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(|task| !task.dismissed)
    }

    /// Whether the tasks from before a reload are known by now.
    pub fn adopted(&self) -> bool {
        self.adopted
    }

    pub fn signed_out(&self) -> bool {
        self.signed_out
    }

    pub fn active_count(&self) -> usize {
        self.tasks.iter().filter(|task| task.is_active()).count()
    }

    /// The first task waiting for a conflict decision.
    pub fn pending_decision(&self) -> Option<&Task> {
        self.tasks.iter().find(|task| task.conflict.is_some())
    }

    pub fn batch_len(&self, batch: u64) -> usize {
        self.tasks.iter().filter(|task| task.batch == batch).count()
    }

    pub(in crate::tasks) fn tasks_mut(&mut self) -> &mut Vec<Task> {
        &mut self.tasks
    }

    pub(in crate::tasks) fn task_mut(&mut self, key: TaskKey) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.key == key)
    }
}

/// A space usage scan only reads: the listing need not reload when it ends.
fn changes_files(spec: &TaskSpec) -> bool {
    !matches!(spec, TaskSpec::Usage { .. })
}

#[cfg(test)]
mod tests {
    use shared::api::conflict::ConflictPolicy;
    use shared::api::error::ErrorBody;
    use shared::api::tasks::Location;

    use super::*;
    use crate::tasks::conflict::ALREADY_EXISTS;

    fn delete(path: &str) -> TaskSpec {
        TaskSpec::Delete {
            source: Location {
                volume: "media".to_owned(),
                path: path.to_owned(),
            },
        }
    }

    fn info(id: &str, path: &str, state: TaskState) -> TaskInfo {
        TaskInfo {
            id: id.to_owned(),
            spec: delete(path),
            state,
            files_done: 0,
            files_total: None,
            bytes_done: 0,
            bytes_total: None,
            skipped: 0,
            final_path: None,
            error: None,
        }
    }

    fn server(code: &str) -> ApiFailure {
        ApiFailure::Server(ErrorBody {
            code: code.to_owned(),
            message: String::new(),
            details: None,
            request_id: Some("req".to_owned()),
        })
    }

    const POLL: Effect = Effect::PollAfter { ms: POLL_EVERY_MS };

    /// Two tasks started and created on the server, as `a` and `b`.
    fn posted() -> TaskList {
        let mut list = TaskList::default();
        list.start(vec![delete("a"), delete("b")]);
        let effects = list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "a", TaskState::Queued)),
        });
        assert_eq!(effects, [POLL]);
        let effects = list.handle(Event::Posted {
            key: TaskKey(1),
            info: Box::new(info("b", "b", TaskState::Queued)),
        });
        assert_eq!(effects, [], "one poll at a time");
        list
    }

    fn states(list: &TaskList) -> Vec<TaskState> {
        list.tasks().iter().map(Task::state).collect()
    }

    #[test]
    fn start_posts_each_task_as_queued() {
        let mut list = TaskList::default();
        let effects = list.start(vec![delete("a"), delete("b")]);
        assert_eq!(
            effects,
            [
                Effect::Post {
                    key: TaskKey(0),
                    spec: delete("a")
                },
                Effect::Post {
                    key: TaskKey(1),
                    spec: delete("b")
                },
            ]
        );
        assert_eq!(states(&list), [TaskState::Queued, TaskState::Queued]);
        assert_eq!(list.active_count(), 2);
    }

    #[test]
    fn poll_updates_progress_and_keeps_polling() {
        let mut list = posted();
        let mut running = info("a", "a", TaskState::Running);
        running.files_total = Some(1);
        let effects = list.handle(Event::Polled(vec![
            running.clone(),
            info("b", "b", TaskState::Queued),
        ]));
        assert_eq!(effects, [POLL]);
        assert_eq!(list.tasks()[0].info, Some(running));
        assert_eq!(states(&list), [TaskState::Running, TaskState::Queued]);
    }

    #[test]
    fn finishing_refreshes_and_idle_stops_polling() {
        let mut list = posted();
        let effects = list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Done),
            info("b", "b", TaskState::Running),
        ]));
        assert_eq!(effects, [Effect::Refresh, POLL]);

        let mut failed = info("b", "b", TaskState::Failed);
        failed.error = Some(ErrorBody {
            code: "permission_denied".to_owned(),
            message: String::new(),
            details: None,
            request_id: Some("req".to_owned()),
        });
        let effects = list.handle(Event::Polled(vec![info("a", "a", TaskState::Done), failed]));
        assert_eq!(effects, [Effect::Refresh]);
        assert_eq!(list.active_count(), 0);
        assert_eq!(list.tasks()[1].failure(), Some(server("permission_denied")));

        assert_eq!(list.handle(Event::Polled(Vec::new())), []);
    }

    #[test]
    fn network_failure_keeps_polling_but_sign_out_stops() {
        let mut list = posted();
        assert_eq!(list.handle(Event::PollFailed(ApiFailure::Network)), [POLL]);
        assert!(!list.signed_out());
        assert_eq!(
            list.handle(Event::PollFailed(server("unauthenticated"))),
            []
        );
        assert_eq!(list.active_count(), 2);
        assert!(list.signed_out());
        list.handle(Event::Polled(vec![info("a", "a", TaskState::Running)]));
        assert!(!list.signed_out());
    }

    #[test]
    fn refused_task_fails_without_polling() {
        let mut list = TaskList::default();
        list.start(vec![delete("a")]);
        let effects = list.handle(Event::PostFailed {
            key: TaskKey(0),
            failure: server("read_only_volume"),
        });
        assert_eq!(effects, []);
        assert_eq!(states(&list), [TaskState::Failed]);
        assert_eq!(list.tasks()[0].failure(), Some(server("read_only_volume")));
        assert_eq!(list.clear_finished(), []);
        assert!(list.tasks().is_empty());
    }

    #[test]
    fn task_missing_from_the_poll_is_lost() {
        let mut list = posted();
        let effects = list.handle(Event::Polled(vec![info("b", "b", TaskState::Running)]));
        assert_eq!(effects, [Effect::Refresh, POLL]);
        assert_eq!(states(&list), [TaskState::Failed, TaskState::Running]);
        assert!(matches!(
            list.tasks()[0].failure(),
            Some(ApiFailure::Server(body)) if body.code == TASK_NOT_FOUND
        ));
    }

    #[test]
    fn task_posted_after_the_poll_was_issued_is_not_lost() {
        let mut list = posted();
        list.start(vec![delete("c")]);
        // The POST answers before the GET that was already on its way.
        let effects = list.handle(Event::Posted {
            key: TaskKey(2),
            info: Box::new(info("c", "c", TaskState::Queued)),
        });
        assert_eq!(effects, []);
        let effects = list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Running),
            info("b", "b", TaskState::Running),
        ]));
        assert_eq!(effects, [POLL]);
        assert_eq!(
            states(&list),
            [TaskState::Running, TaskState::Running, TaskState::Queued]
        );

        // The next poll was issued knowing `c`: missing from it now, `c` is lost.
        let effects = list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Running),
            info("b", "b", TaskState::Running),
        ]));
        assert_eq!(effects, [Effect::Refresh, POLL]);
        assert_eq!(list.tasks()[2].state(), TaskState::Failed);
    }

    #[test]
    fn adopts_the_running_tasks_of_an_earlier_page_and_polls() {
        let mut list = TaskList::default();
        let effects = list.adopt(vec![
            info("a", "a", TaskState::Running),
            info("b", "b", TaskState::Done),
            info("c", "c", TaskState::Queued),
        ]);
        assert_eq!(effects, [POLL]);
        let ids: Vec<&str> = list
            .tasks()
            .iter()
            .filter_map(|task| Some(task.info.as_ref()?.id.as_str()))
            .collect();
        assert_eq!(ids, ["a", "c"]);
        assert_eq!(list.tasks()[0].spec, delete("a"));
        assert_ne!(list.tasks()[0].batch, list.tasks()[1].batch);
        assert!(!list.tasks()[0].allows_replace());

        let effects = list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Done),
            info("c", "c", TaskState::Done),
        ]));
        assert_eq!(effects, [Effect::Refresh]);
        assert_eq!(list.cancel(list.tasks()[0].key), []);
    }

    #[test]
    fn adopting_nothing_active_does_not_poll() {
        let mut list = TaskList::default();
        assert_eq!(list.adopt(vec![info("a", "a", TaskState::Done)]), []);
        assert!(list.tasks().is_empty());
    }

    #[test]
    fn adoption_and_a_post_of_the_same_task_keep_one_row() {
        // Started before the adoption answer: the list already has it.
        let mut list = posted();
        assert_eq!(list.adopt(vec![info("a", "a", TaskState::Running)]), []);
        assert_eq!(list.tasks().len(), 2);

        // Adopted before its own POST answered.
        let mut list = TaskList::default();
        list.start(vec![delete("a")]);
        list.adopt(vec![info("a", "a", TaskState::Queued)]);
        list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "a", TaskState::Running)),
        });
        let keys: Vec<TaskKey> = list.tasks().iter().map(|task| task.key).collect();
        assert_eq!(keys, [TaskKey(0)]);
    }

    #[test]
    fn cancel_deletes_on_the_server() {
        let mut list = posted();
        assert_eq!(
            list.cancel(TaskKey(0)),
            [Effect::Delete { id: "a".to_owned() }]
        );
        list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Cancelled),
            info("b", "b", TaskState::Done),
        ]));
        assert_eq!(list.cancel(TaskKey(0)), [], "already finished");
    }

    #[test]
    fn failed_cancel_shows_until_the_next_try() {
        let mut list = posted();
        list.cancel(TaskKey(0));
        list.handle(Event::DeleteFailed {
            id: "a".to_owned(),
            failure: ApiFailure::Network,
        });
        assert_eq!(list.tasks()[0].cancel_failure(), Some(&ApiFailure::Network));
        assert!(list.tasks()[0].is_active());
        assert_eq!(
            list.cancel(TaskKey(0)),
            [Effect::Delete { id: "a".to_owned() }]
        );
        assert_eq!(list.tasks()[0].cancel_failure(), None);

        list.handle(Event::DeleteFailed {
            id: "b".to_owned(),
            failure: server("task_not_found"),
        });
        assert_eq!(list.tasks()[1].state(), TaskState::Failed);
    }

    #[test]
    fn cancel_before_the_server_has_it() {
        let mut list = TaskList::default();
        list.start(vec![delete("a")]);
        assert_eq!(list.cancel(TaskKey(0)), []);
        let effects = list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "a", TaskState::Queued)),
        });
        assert_eq!(effects, [Effect::Delete { id: "a".to_owned() }, POLL]);
    }

    #[test]
    fn clear_finished_dismisses_them_on_the_server() {
        let mut list = posted();
        list.start(vec![delete("c")]);
        list.handle(Event::Polled(vec![
            info("a", "a", TaskState::Done),
            info("b", "b", TaskState::Running),
        ]));
        assert_eq!(
            list.clear_finished(),
            [Effect::Delete { id: "a".to_owned() }]
        );
        let left: Vec<TaskKey> = list.tasks().iter().map(|task| task.key).collect();
        assert_eq!(left, [TaskKey(1), TaskKey(2)]);
    }

    fn copy(path: &str, conflict: ConflictPolicy) -> TaskSpec {
        TaskSpec::Copy {
            source: Location {
                volume: "media".to_owned(),
                path: path.to_owned(),
            },
            dest_dir: Location {
                volume: "documents".to_owned(),
                path: String::new(),
            },
            conflict,
        }
    }

    fn exists(path: &str, kind: &str) -> ApiFailure {
        ApiFailure::Server(ErrorBody {
            code: ALREADY_EXISTS.to_owned(),
            message: String::new(),
            details: Some(serde_json::json!({
                "volume": "documents",
                "path": path,
                "kind": kind,
            })),
            request_id: None,
        })
    }

    fn post(key: u64, path: &str, conflict: ConflictPolicy) -> Effect {
        Effect::Post {
            key: TaskKey(key),
            spec: copy(path, conflict),
        }
    }

    fn refuse(list: &mut TaskList, key: u64, failure: ApiFailure) -> Vec<Effect> {
        list.handle(Event::PostFailed {
            key: TaskKey(key),
            failure,
        })
    }

    fn waiting(list: &TaskList) -> Vec<TaskKey> {
        list.tasks()
            .iter()
            .filter(|task| task.conflict().is_some())
            .map(|task| task.key)
            .collect()
    }

    /// `a`, `b` and `c` copied in one action; `d` in another.
    fn copies() -> TaskList {
        let mut list = TaskList::default();
        let effects = list.start_transfers(vec![
            (copy("a", ConflictPolicy::Fail), EntryKind::File),
            (copy("b", ConflictPolicy::Fail), EntryKind::File),
            (copy("c", ConflictPolicy::Fail), EntryKind::File),
        ]);
        assert_eq!(effects.len(), 3);
        list.start_transfers(vec![(copy("d", ConflictPolicy::Fail), EntryKind::File)]);
        list
    }

    #[test]
    fn conflict_waits_for_a_decision() {
        let mut list = copies();
        assert_eq!(refuse(&mut list, 1, exists("b", "file")), []);

        let task = list.pending_decision().unwrap();
        assert_eq!(task.key, TaskKey(1));
        assert_eq!(
            task.conflict(),
            Some(&Conflict {
                volume: "documents".to_owned(),
                path: "b".to_owned(),
                kind: EntryKind::File,
            })
        );
        assert!(task.allows_replace());
        assert_eq!(task.state(), TaskState::Queued);
        assert_eq!(list.batch_len(task.batch), 3);
        assert_eq!(list.active_count(), 4);
        assert_eq!(list.clear_finished(), []);
        assert_eq!(list.tasks().len(), 4);
    }

    #[test]
    fn apply_to_all_resends_the_batch_with_the_policy() {
        let mut list = copies();
        refuse(&mut list, 0, exists("a", "file"));
        refuse(&mut list, 1, exists("b", "file"));
        refuse(&mut list, 3, exists("d", "file"));

        let effects = list.decide(TaskKey(0), Decision::KeepBoth, true);
        assert_eq!(
            effects,
            [
                post(0, "a", ConflictPolicy::KeepBoth),
                post(1, "b", ConflictPolicy::KeepBoth),
            ]
        );
        assert_eq!(
            waiting(&list),
            [TaskKey(3)],
            "another action asks on its own"
        );

        // A later conflict of the batch gets the same answer.
        let effects = refuse(&mut list, 2, exists("c", "file"));
        assert_eq!(effects, [post(2, "c", ConflictPolicy::KeepBoth)]);
        assert_eq!(list.tasks()[2].spec, copy("c", ConflictPolicy::KeepBoth));
    }

    #[test]
    fn single_decision_and_cancel_drop_only_that_task() {
        let mut list = copies();
        refuse(&mut list, 0, exists("a", "file"));
        refuse(&mut list, 1, exists("b", "file"));

        assert_eq!(list.decide(TaskKey(0), Decision::Cancel, false), []);
        assert_eq!(waiting(&list), [TaskKey(1)]);
        assert!(list.tasks().iter().all(|task| task.key != TaskKey(0)));

        assert_eq!(
            list.decide(TaskKey(1), Decision::Replace, false),
            [post(1, "b", ConflictPolicy::Replace)]
        );
        assert!(waiting(&list).is_empty());

        // A later conflict of the batch still asks.
        refuse(&mut list, 2, exists("c", "file"));
        assert_eq!(waiting(&list), [TaskKey(2)]);
        assert_eq!(list.decide(TaskKey(9), Decision::KeepBoth, false), []);
    }

    #[test]
    fn replace_only_suits_a_file_over_a_file() {
        let mut list = TaskList::default();
        list.start_transfers(vec![
            (copy("a", ConflictPolicy::Fail), EntryKind::File),
            (copy("F", ConflictPolicy::Fail), EntryKind::Dir),
            (copy("b", ConflictPolicy::Fail), EntryKind::File),
        ]);
        refuse(&mut list, 0, exists("a", "file"));
        refuse(&mut list, 1, exists("F", "dir"));
        refuse(&mut list, 2, exists("b", "dir"));
        let allows: Vec<bool> = list.tasks().iter().map(Task::allows_replace).collect();
        assert_eq!(allows, [true, false, false]);

        let effects = list.decide(TaskKey(0), Decision::Replace, true);
        assert_eq!(effects, [post(0, "a", ConflictPolicy::Replace)]);
        assert_eq!(waiting(&list), [TaskKey(1), TaskKey(2)]);
    }

    #[test]
    fn other_refusals_fail_the_task() {
        let mut list = copies();
        refuse(&mut list, 0, server("invalid_destination"));
        assert_eq!(list.tasks()[0].state(), TaskState::Failed);

        // Already sent with a policy: the server answer stands.
        let mut list = TaskList::default();
        list.start_transfers(vec![(copy("a", ConflictPolicy::KeepBoth), EntryKind::File)]);
        refuse(&mut list, 0, exists("a", "file"));
        assert_eq!(list.tasks()[0].state(), TaskState::Failed);
        assert!(waiting(&list).is_empty());
    }

    #[test]
    fn batch_answers_are_forgotten_once_the_batch_ends() {
        let mut list = copies();
        refuse(&mut list, 0, exists("a", "file"));
        list.decide(TaskKey(0), Decision::KeepBoth, true);
        assert_eq!(list.decisions.len(), 1);
        for key in 0..3 {
            refuse(&mut list, key, server("invalid_destination"));
        }
        assert!(list.decisions.is_empty());

        let mut list = copies();
        for key in 0..3 {
            refuse(&mut list, key, exists("a", "file"));
        }
        list.decide(TaskKey(0), Decision::Cancel, true);
        assert!(list.decisions.is_empty());
    }

    #[test]
    fn cancelling_a_waiting_task_drops_it() {
        let mut list = copies();
        refuse(&mut list, 0, exists("a", "file"));
        assert_eq!(list.cancel(TaskKey(0)), []);
        assert_eq!(list.tasks().len(), 3);

        list.cancel(TaskKey(1));
        refuse(&mut list, 1, exists("b", "file"));
        assert_eq!(list.tasks().len(), 2);
        assert_eq!(list.pending_decision(), None);
    }
}
