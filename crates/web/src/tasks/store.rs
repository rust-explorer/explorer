use std::collections::HashMap;

use shared::api::browse::EntryKind;
use shared::api::conflict::ConflictPolicy;
use shared::api::error::ErrorBody;
use shared::api::tasks::{Location, TaskInfo, TaskSpec, TaskState};

use super::conflict::{Conflict, policy_of, with_policy};
use crate::api::ApiFailure;
use crate::conflict::Decision;

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
    /// Its page opens once the server gives it an id.
    pub(super) open_when_posted: bool,
    /// When this tab saw it finish.
    finished_at_ms: Option<i64>,
    /// Cleared from the activity panel, but still known for its result.
    dismissed: bool,
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
    /// Shows the result page of the usage task `id`.
    Open { id: String, source: Location },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Posted { key: TaskKey, info: Box<TaskInfo> },
    PostFailed { key: TaskKey, failure: ApiFailure },
    Polled(Vec<TaskInfo>),
    PollFailed(ApiFailure),
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
}

impl TaskList {
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The tasks the activity panel lists.
    pub fn shown(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(|task| !task.dismissed)
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

    /// Starts one batch of tasks whose source kinds are unknown.
    pub fn start(&mut self, specs: Vec<TaskSpec>) -> Vec<Effect> {
        self.start_batch(specs.into_iter().map(|spec| (spec, None)).collect())
    }

    /// Starts one batch of copies or moves of entries of known kinds.
    pub fn start_transfers(&mut self, specs: Vec<(TaskSpec, EntryKind)>) -> Vec<Effect> {
        self.start_batch(
            specs
                .into_iter()
                .map(|(spec, kind)| (spec, Some(kind)))
                .collect(),
        )
    }

    pub(super) fn start_batch(&mut self, specs: Vec<(TaskSpec, Option<EntryKind>)>) -> Vec<Effect> {
        let batch = self.next_batch;
        self.next_batch += 1;
        specs
            .into_iter()
            .map(|(spec, source_kind)| {
                let key = TaskKey(self.next_key);
                self.next_key += 1;
                self.tasks.push(Task {
                    key,
                    spec: spec.clone(),
                    info: None,
                    lost: None,
                    cancel_requested: false,
                    batch,
                    source_kind,
                    conflict: None,
                    open_when_posted: false,
                    finished_at_ms: None,
                    dismissed: false,
                });
                Effect::Post { key, spec }
            })
            .collect()
    }

    /// Takes over the queued and running tasks the server has for this user, such as those an
    /// earlier page started before a reload.
    pub fn adopt(&mut self, infos: Vec<TaskInfo>) -> Vec<Effect> {
        for info in infos {
            let known = self
                .tasks
                .iter()
                .any(|task| task.server_id() == Some(info.id.as_str()));
            if known || !info.state.is_active() {
                continue;
            }
            let (key, batch) = (TaskKey(self.next_key), self.next_batch);
            self.next_key += 1;
            self.next_batch += 1;
            self.tasks.push(Task {
                key,
                spec: info.spec.clone(),
                info: Some(info),
                lost: None,
                cancel_requested: false,
                batch,
                source_kind: None,
                conflict: None,
                open_when_posted: false,
                finished_at_ms: None,
                dismissed: false,
            });
        }
        let mut effects = Vec::new();
        self.keep_polling(&mut effects);
        effects
    }

    /// Answers the conflict of `key`, or of every task of its batch waiting with one that the
    /// decision suits; a later conflict in the batch then gets the same answer.
    pub fn decide(&mut self, key: TaskKey, decision: Decision, apply_to_all: bool) -> Vec<Effect> {
        let Some(batch) = self
            .tasks
            .iter()
            .find(|task| task.key == key && task.conflict.is_some())
            .map(|task| task.batch)
        else {
            return Vec::new();
        };
        let keys: Vec<TaskKey> = if apply_to_all {
            self.decisions.insert(batch, decision);
            self.tasks
                .iter()
                .filter(|task| task.batch == batch && task.conflict.is_some())
                .filter(|task| suits(task, decision))
                .map(|task| task.key)
                .collect()
        } else {
            vec![key]
        };
        let mut effects = Vec::new();
        for key in keys {
            self.apply(key, decision, &mut effects);
        }
        effects
    }

    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::Posted { key, info } => {
                // An adoption may have listed it before this answer.
                self.tasks
                    .retain(|task| task.key == key || task.server_id() != Some(info.id.as_str()));
                if let Some(task) = self.task_mut(key) {
                    if task.cancel_requested && info.state.is_active() {
                        effects.push(Effect::Delete {
                            id: info.id.clone(),
                        });
                    }
                    if std::mem::take(&mut task.open_when_posted) {
                        effects.push(Effect::Open {
                            id: info.id.clone(),
                            source: task.spec.source().clone(),
                        });
                    }
                    let finished = !info.state.is_active();
                    task.info = Some(*info);
                    if finished {
                        effects.push(Effect::Refresh);
                    }
                }
            }
            Event::PostFailed { key, failure } => self.refused(key, failure, &mut effects),
            Event::Polled(infos) => {
                self.polling = false;
                if self.update(&infos) {
                    effects.push(Effect::Refresh);
                }
            }
            Event::PollFailed(failure) => {
                self.polling = false;
                if failure.is_unauthenticated() {
                    return effects;
                }
            }
        }
        self.keep_polling(&mut effects);
        effects
    }

    /// The task stops once the server notices; before the server has it, once it does.
    pub fn cancel(&mut self, key: TaskKey) -> Vec<Effect> {
        let Some(task) = self.task_mut(key).filter(|task| task.is_active()) else {
            return Vec::new();
        };
        if task.conflict.is_some() {
            self.tasks.retain(|task| task.key != key);
            return Vec::new();
        }
        if let Some(id) = task.server_id() {
            return vec![Effect::Delete { id: id.to_owned() }];
        }
        task.cancel_requested = true;
        Vec::new()
    }

    /// Forgets the finished tasks, here and on the server. A finished scan only leaves the
    /// panel: its result stays on the server, which drops old ones by itself.
    pub fn clear_finished(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.tasks.retain_mut(|task| {
            if task.is_active() {
                return true;
            }
            if matches!(task.spec, TaskSpec::Usage { .. }) && task.state() == TaskState::Done {
                task.dismissed = true;
                return true;
            }
            if let Some(id) = task.server_id() {
                effects.push(Effect::Delete { id: id.to_owned() });
            }
            false
        });
        effects
    }

    /// Notes `now_ms` as the finish time of the tasks that just finished.
    pub fn stamp_finished(&mut self, now_ms: i64) {
        for task in &mut self.tasks {
            if !task.is_active() && task.finished_at_ms.is_none() {
                task.finished_at_ms = Some(now_ms);
            }
        }
    }

    /// Applies a poll; true when a task finished.
    fn update(&mut self, infos: &[TaskInfo]) -> bool {
        let mut finished = false;
        for task in &mut self.tasks {
            let Some(current) = task.info.as_ref().filter(|_| task.is_active()) else {
                continue;
            };
            if let Some(info) = infos.iter().find(|info| info.id == current.id) {
                finished |= !info.state.is_active();
                task.info = Some(info.clone());
            } else if self.polled.contains(&current.id) {
                // Lost with a server restart.
                task.lost = Some(ApiFailure::Server(ErrorBody {
                    code: TASK_NOT_FOUND.to_owned(),
                    message: String::new(),
                    details: None,
                    request_id: None,
                }));
                finished = true;
            }
        }
        finished
    }

    fn keep_polling(&mut self, effects: &mut Vec<Effect>) {
        if self.polling {
            return;
        }
        let waiting: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.is_active())
            .filter_map(|task| task.server_id().map(str::to_owned))
            .collect();
        if !waiting.is_empty() {
            self.polling = true;
            self.polled = waiting;
            effects.push(Effect::PollAfter { ms: POLL_EVERY_MS });
        }
    }

    /// A copy or move refused because its destination exists waits for a decision, unless
    /// its batch already has one; any other refusal fails the task.
    fn refused(&mut self, key: TaskKey, failure: ApiFailure, effects: &mut Vec<Effect>) {
        let decision = self
            .tasks
            .iter()
            .find(|task| task.key == key)
            .and_then(|task| self.decisions.get(&task.batch).copied());
        let Some(task) = self.task_mut(key) else {
            return;
        };
        let conflict =
            Conflict::of(&failure).filter(|_| policy_of(&task.spec) == Some(ConflictPolicy::Fail));
        let Some(conflict) = conflict else {
            task.lost = Some(failure);
            return;
        };
        if task.cancel_requested {
            self.tasks.retain(|task| task.key != key);
            return;
        }
        task.conflict = Some(conflict);
        if let Some(decision) = decision.filter(|&decision| suits(task, decision)) {
            self.apply(key, decision, effects);
        }
    }

    /// Resends the task with the decision's policy, or drops it on Cancel.
    fn apply(&mut self, key: TaskKey, decision: Decision, effects: &mut Vec<Effect>) {
        match decision.policy() {
            None => self.tasks.retain(|task| task.key != key),
            Some(conflict) => {
                if let Some(task) = self.task_mut(key) {
                    task.conflict = None;
                    task.spec = with_policy(&task.spec, conflict);
                    effects.push(Effect::Post {
                        key,
                        spec: task.spec.clone(),
                    });
                }
            }
        }
    }

    pub(super) fn tasks_mut(&mut self) -> &mut Vec<Task> {
        &mut self.tasks
    }

    pub(super) fn task_mut(&mut self, key: TaskKey) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.key == key)
    }
}

/// Replace only suits a file over a file.
fn suits(task: &Task, decision: Decision) -> bool {
    decision != Decision::Replace || task.allows_replace()
}

#[cfg(test)]
mod tests {
    use shared::api::tasks::Location;

    use super::super::conflict::ALREADY_EXISTS;
    use super::*;

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
        assert_eq!(
            list.handle(Event::PollFailed(server("unauthenticated"))),
            []
        );
        assert_eq!(list.active_count(), 2);
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
