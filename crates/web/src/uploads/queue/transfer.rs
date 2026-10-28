use shared::api::conflict::ConflictPolicy;

use super::item::Phase;
use super::{Effect, ItemKey, Queue, Status};
use crate::api::ApiFailure;
use crate::conflict::Decision;
use crate::uploads::retry_delay_ms;

const ALREADY_EXISTS: &str = "already_exists";
const UPLOAD_NOT_FOUND: &str = "upload_not_found";
const UPLOAD_BUSY: &str = "upload_busy";
const OFFSET_MISMATCH: &str = "offset_mismatch";
/// The server's only JSON 5xx besides 507 `insufficient_storage`.
const INTERNAL: &str = "internal";

fn is_transient(failure: &ApiFailure) -> bool {
    match failure {
        ApiFailure::Network => true,
        ApiFailure::Http(status) => *status >= 500 && *status != 507,
        ApiFailure::Server(body) => {
            matches!(body.code.as_str(), INTERNAL | UPLOAD_BUSY | OFFSET_MISMATCH)
        }
    }
}

impl Queue {
    /// Continues from the server's `offset`; only a PATCH that moved it resets the backoff.
    pub(super) fn resume(
        &mut self,
        key: ItemKey,
        offset: u64,
        patched: bool,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self.in_flight(key) else {
            return;
        };
        let item = &mut self.items[index];
        if patched && offset > item.confirmed {
            item.attempts = 0;
        }
        item.confirmed = offset;
        let Some(url) = item.upload_url.clone() else {
            self.finish(index, Status::Failed(ApiFailure::Network), now_ms, effects);
            return;
        };
        if !patched && offset >= item.size() {
            // A HEAD also reports every byte when the commit failed after the last PATCH; an
            // empty PATCH at the end commits it, or confirms an upload already committed.
            effects.push(item.patch(url, item.size()));
            return;
        }
        self.advance(index, url, offset, now_ms, effects);
    }

    pub(super) fn advance(
        &mut self,
        index: usize,
        url: String,
        offset: u64,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let item = &mut self.items[index];
        if offset >= item.size() {
            item.sent = item.size();
            self.finish(index, Status::Done, now_ms, effects);
        } else {
            effects.push(item.patch(url, offset));
        }
    }

    pub(super) fn fail(
        &mut self,
        index: usize,
        failure: ApiFailure,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let decision = self.decisions.get(&self.items[index].batch).copied();
        let item = &mut self.items[index];
        if is_transient(&failure) {
            item.attempts += 1;
            let at_ms = now_ms + retry_delay_ms(item.attempts);
            item.status = Status::Retrying {
                attempt: item.attempts,
                at_ms,
                connection_lost: failure == ApiFailure::Network,
            };
            item.phase = Phase::Idle;
            effects.push(Effect::WakeAt { at_ms });
            return;
        }
        let code = match &failure {
            ApiFailure::Server(body) => body.code.as_str(),
            _ => "",
        };
        match (code, item.phase) {
            (ALREADY_EXISTS, Phase::Creating) if item.conflict == ConflictPolicy::Fail => {
                match decision.map(Decision::policy) {
                    Some(Some(conflict)) => {
                        item.conflict = conflict;
                        effects.push(item.create());
                    }
                    Some(None) => self.finish(index, Status::Skipped, now_ms, effects),
                    None => {
                        item.status = Status::NeedsDecision;
                        item.phase = Phase::Idle;
                    }
                }
            }
            (UPLOAD_NOT_FOUND, Phase::Heading | Phase::Patching) if !item.restarted => {
                item.restarted = true;
                item.upload_url = None;
                item.sent = 0;
                item.confirmed = 0;
                effects.push(item.create());
            }
            _ => {
                // The server keeps the staged file until its sweep unless told otherwise.
                if let Some(url) = item.upload_url.take()
                    && code != UPLOAD_NOT_FOUND
                {
                    effects.push(Effect::Delete { url });
                }
                self.finish(index, Status::Failed(failure), now_ms, effects);
            }
        }
    }
}
