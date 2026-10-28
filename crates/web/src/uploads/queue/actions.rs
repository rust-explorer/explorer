use super::{Effect, ItemKey, Queue, Status};
use crate::conflict::Decision;

impl Queue {
    pub(super) fn decide(
        &mut self,
        key: ItemKey,
        decision: Decision,
        apply_to_all: bool,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self
            .index(key)
            .filter(|&index| self.items[index].status == Status::NeedsDecision)
        else {
            return;
        };
        let batch = self.items[index].batch;
        let targets: Vec<usize> = if apply_to_all {
            self.decisions.insert(batch, decision);
            (0..self.items.len())
                .filter(|&i| {
                    self.items[i].batch == batch && self.items[i].status == Status::NeedsDecision
                })
                .collect()
        } else {
            vec![index]
        };
        for target in targets {
            match decision.policy() {
                Some(conflict) => {
                    let item = &mut self.items[target];
                    item.conflict = conflict;
                    item.status = Status::Queued;
                }
                None => self.finish(target, Status::Skipped, now_ms, effects),
            }
        }
    }

    pub(super) fn cancel(&mut self, key: ItemKey, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(index) = self.index(key) else {
            return;
        };
        let item = &self.items[index];
        if !item.status.is_active() {
            return;
        }
        if item.status == Status::Uploading {
            effects.push(Effect::Abort { key });
        }
        if let Some(url) = item.upload_url.clone() {
            effects.push(Effect::Delete { key, url });
        }
        self.finish(index, Status::Cancelled, now_ms, effects);
    }

    /// The upload was complete on the server before the cancel reached it.
    pub(super) fn cancel_too_late(&mut self, key: ItemKey, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(index) = self
            .index(key)
            .filter(|&index| self.items[index].status == Status::Cancelled)
        else {
            return;
        };
        let item = &mut self.items[index];
        item.sent = item.size();
        self.finish(index, Status::Done, now_ms, effects);
    }
}
