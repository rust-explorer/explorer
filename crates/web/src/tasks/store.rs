use shared::api::error::ErrorBody;
use shared::api::tasks::{TaskInfo, TaskSpec, TaskState};

use crate::api::ApiFailure;

pub const POLL_EVERY_MS: u32 = 1000;
const TASK_NOT_FOUND: &str = "task_not_found";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskKey(pub u64);

/// A task this tab started.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub key: TaskKey,
    pub spec: TaskSpec,
    /// The server's view, once it created the task.
    pub info: Option<TaskInfo>,
    /// The server refused the task, or no longer knows it.
    lost: Option<ApiFailure>,
    cancel_requested: bool,
}

impl Task {
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

    /// The server's id, while the server has the task.
    fn server_id(&self) -> Option<&str> {
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
}

impl TaskList {
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn active_count(&self) -> usize {
        self.tasks.iter().filter(|task| task.is_active()).count()
    }

    pub fn start(&mut self, specs: Vec<TaskSpec>) -> Vec<Effect> {
        specs
            .into_iter()
            .map(|spec| {
                let key = TaskKey(self.next_key);
                self.next_key += 1;
                self.tasks.push(Task {
                    key,
                    spec: spec.clone(),
                    info: None,
                    lost: None,
                    cancel_requested: false,
                });
                Effect::Post { key, spec }
            })
            .collect()
    }

    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::Posted { key, info } => {
                if let Some(task) = self.task_mut(key) {
                    if task.cancel_requested && info.state.is_active() {
                        effects.push(Effect::Delete {
                            id: info.id.clone(),
                        });
                    }
                    let finished = !info.state.is_active();
                    task.info = Some(*info);
                    if finished {
                        effects.push(Effect::Refresh);
                    }
                }
            }
            Event::PostFailed { key, failure } => {
                if let Some(task) = self.task_mut(key) {
                    task.lost = Some(failure);
                }
            }
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
        if let Some(id) = task.server_id() {
            return vec![Effect::Delete { id: id.to_owned() }];
        }
        task.cancel_requested = true;
        Vec::new()
    }

    /// Forgets the finished tasks, here and on the server.
    pub fn clear_finished(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.tasks.retain(|task| {
            if task.is_active() {
                return true;
            }
            if let Some(id) = task.server_id() {
                effects.push(Effect::Delete { id: id.to_owned() });
            }
            false
        });
        effects
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

    fn task_mut(&mut self, key: TaskKey) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.key == key)
    }
}

#[cfg(test)]
mod tests {
    use shared::api::tasks::Location;

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
}
