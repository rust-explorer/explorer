use shared::api::browse::EntryKind;
use shared::api::tasks::{TaskInfo, TaskSpec, TaskState};

use super::{Effect, TASK_NOT_FOUND, Task, TaskKey, TaskList};
use crate::api::ApiFailure;

impl TaskList {
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

    pub(in crate::tasks) fn start_batch(
        &mut self,
        specs: Vec<(TaskSpec, Option<EntryKind>)>,
    ) -> Vec<Effect> {
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
                    open_at: None,
                    finished_at_ms: None,
                    finished_before: false,
                    dismissed: false,
                    result_dropped: false,
                    cancel_failure: None,
                });
                Effect::Post { key, spec }
            })
            .collect()
    }

    /// Takes over the queued and running tasks the server has for this user, such as those an
    /// earlier page started before a reload. Finished scans are taken too, for their result,
    /// but stay out of the panel like the other finished tasks.
    pub fn adopt(&mut self, infos: Vec<TaskInfo>) -> Vec<Effect> {
        for info in infos {
            let known = self
                .tasks
                .iter()
                .any(|task| task.server_id() == Some(info.id.as_str()));
            let finished_scan =
                matches!(info.spec, TaskSpec::Usage { .. }) && info.state == TaskState::Done;
            if known || !(info.state.is_active() || finished_scan) {
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
                open_at: None,
                finished_at_ms: None,
                finished_before: finished_scan,
                dismissed: finished_scan,
                result_dropped: false,
                cancel_failure: None,
            });
        }
        self.adopted = true;
        let mut effects = Vec::new();
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
            self.forget_ended_batches();
            return Vec::new();
        }
        task.cancel_failure = None;
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
        self.drop_stale_scans();
        self.forget_ended_batches();
        effects
    }

    /// The server no longer has the result of the finished scan `id`, such as when scans from
    /// another tab pushed it out: the next analysis of its folder starts a new one.
    pub fn result_gone(&mut self, id: &str) {
        if let Some(task) = self.tasks.iter_mut().find(|task| {
            task.server_id() == Some(id)
                && matches!(task.spec, TaskSpec::Usage { .. })
                && task.state() == TaskState::Done
        }) {
            task.result_dropped = true;
        }
    }

    /// Notes `now_ms` as the finish time of the tasks that just finished.
    pub fn stamp_finished(&mut self, now_ms: i64) {
        for task in &mut self.tasks {
            if !task.is_active() && task.finished_at_ms.is_none() && !task.finished_before {
                task.finished_at_ms = Some(now_ms);
            }
        }
    }

    /// A task the server no longer knows is lost; any other failure is shown on its row, and
    /// the task goes on until the user cancels again.
    pub(super) fn cancel_failed(&mut self, id: &str, failure: ApiFailure) {
        if failure.is_unauthenticated() {
            self.signed_out = true;
            return;
        }
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.is_active() && task.server_id() == Some(id))
        else {
            return;
        };
        if matches!(&failure, ApiFailure::Server(body) if body.code == TASK_NOT_FOUND) {
            task.lost = Some(failure);
        } else {
            task.cancel_failure = Some(failure);
        }
    }
}
