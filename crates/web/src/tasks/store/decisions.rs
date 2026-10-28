use shared::api::conflict::ConflictPolicy;

use super::{Effect, Task, TaskKey, TaskList};
use crate::api::ApiFailure;
use crate::conflict::Decision;
use crate::tasks::conflict::{Conflict, policy_of, with_policy};

impl TaskList {
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
        self.forget_ended_batches();
        effects
    }

    /// Drops the answers of batches with no task left to answer for.
    pub(super) fn forget_ended_batches(&mut self) {
        let tasks = &self.tasks;
        self.decisions.retain(|&batch, _| {
            tasks
                .iter()
                .any(|task| task.batch == batch && task.is_active())
        });
    }

    /// A copy or move refused because its destination exists waits for a decision, unless
    /// its batch already has one; any other refusal fails the task.
    pub(super) fn refused(&mut self, key: TaskKey, failure: ApiFailure, effects: &mut Vec<Effect>) {
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
    pub(super) fn apply(&mut self, key: TaskKey, decision: Decision, effects: &mut Vec<Effect>) {
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
}

/// Replace only suits a file over a file.
fn suits(task: &Task, decision: Decision) -> bool {
    decision != Decision::Replace || task.allows_replace()
}
