use super::item::Phase;
use super::{Effect, Queue, Status};
use crate::uploads::REFRESH_EVERY_MS;

impl Queue {
    /// Ends an item, and its batch's conflict answer with its last item. Refreshes its volume's
    /// listing when completions are due; a throttled completion schedules the refresh for when
    /// the throttle ends.
    pub(super) fn finish(
        &mut self,
        index: usize,
        status: Status,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let done = status == Status::Done;
        let item = &mut self.items[index];
        item.status = status;
        item.phase = Phase::Idle;
        let (batch, volume) = (item.batch, item.volume().to_owned());
        if !self
            .items
            .iter()
            .any(|item| item.batch == batch && item.status.is_active())
        {
            self.decisions.remove(&batch);
        }
        let first_unrefreshed = done && self.unrefreshed.insert(volume.clone());
        if !self.unrefreshed.contains(&volume) {
            return;
        }
        let idle = !self
            .items
            .iter()
            .any(|item| item.status.is_active() && item.volume() == volume);
        if idle || (done && self.refresh_due(&volume, now_ms)) {
            self.refresh(volume, now_ms, effects);
        } else if first_unrefreshed && let Some(&last) = self.last_refresh.get(&volume) {
            effects.push(Effect::WakeAt {
                at_ms: last + REFRESH_EVERY_MS,
            });
        }
    }

    pub(super) fn refresh_due(&self, volume: &str, now_ms: i64) -> bool {
        self.last_refresh
            .get(volume)
            .is_none_or(|&last| now_ms - last >= REFRESH_EVERY_MS)
    }

    pub(super) fn refresh(&mut self, volume: String, now_ms: i64, effects: &mut Vec<Effect>) {
        self.unrefreshed.remove(&volume);
        self.last_refresh.insert(volume.clone(), now_ms);
        effects.push(Effect::Refresh { volume });
    }
}
