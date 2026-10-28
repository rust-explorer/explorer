use shared::api::error::ErrorBody;
use shared::api::tasks::{Location, TaskInfo};

use super::{Effect, Event, POLL_EVERY_MS, TASK_NOT_FOUND, TaskList, changes_files};
use crate::api::ApiFailure;

impl TaskList {
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
                    if let Some(path) = task.open_at.take() {
                        effects.push(Effect::Open {
                            id: info.id.clone(),
                            at: Location {
                                volume: task.spec.source().volume.clone(),
                                path,
                            },
                        });
                    }
                    let finished = !info.state.is_active() && changes_files(&task.spec);
                    task.info = Some(*info);
                    if finished {
                        effects.push(Effect::Refresh);
                    }
                }
            }
            Event::PostFailed { key, failure } => self.refused(key, failure, &mut effects),
            Event::Polled(infos) => {
                self.polling = false;
                self.signed_out = false;
                if self.update(&infos) {
                    effects.push(Effect::Refresh);
                }
            }
            Event::PollFailed(failure) => {
                self.polling = false;
                if failure.is_unauthenticated() {
                    self.signed_out = true;
                    return effects;
                }
            }
            Event::DeleteFailed { id, failure } => self.cancel_failed(&id, failure),
        }
        self.keep_polling(&mut effects);
        effects
    }

    /// Applies a poll; true when a task that changes files finished.
    pub(super) fn update(&mut self, infos: &[TaskInfo]) -> bool {
        let mut finished = false;
        for task in &mut self.tasks {
            let Some(current) = task.info.as_ref().filter(|_| task.is_active()) else {
                continue;
            };
            if let Some(info) = infos.iter().find(|info| info.id == current.id) {
                finished |= !info.state.is_active() && changes_files(&task.spec);
                task.info = Some(info.clone());
            } else if self.polled.contains(&current.id) {
                // Lost with a server restart.
                task.lost = Some(ApiFailure::Server(ErrorBody {
                    code: TASK_NOT_FOUND.to_owned(),
                    message: String::new(),
                    details: None,
                    request_id: None,
                }));
                finished |= changes_files(&task.spec);
            }
        }
        finished
    }

    pub(super) fn keep_polling(&mut self, effects: &mut Vec<Effect>) {
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
}
