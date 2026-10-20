mod controller;
mod plan;
mod queue;
mod speed;
#[cfg(target_arch = "wasm32")]
mod transport;

pub use controller::{Revision, Uploads, use_uploads, use_uploads_provider};
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_imports))]
pub use plan::{BatchPlan, FileSpec, PickedFile, plan_batch};
pub use queue::{Decision, Effect, Event, Item, ItemKey, Kind, Queue, Status};
pub use speed::SpeedMeter;

pub const CHUNK_SIZE: u64 = 16 * 1024 * 1024;
pub const MAX_PARALLEL: usize = 3;
pub const REFRESH_EVERY_MS: i64 = 2000;
const MAX_RETRY_DELAY_MS: i64 = 30_000;

/// Delay before retry `attempt` (counted from 1) after a transient failure.
pub fn retry_delay_ms(attempt: u32) -> i64 {
    let doublings = attempt.saturating_sub(1).min(5);
    (1000_i64 << doublings).min(MAX_RETRY_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delays() {
        let delays: Vec<i64> = (1..=7).map(retry_delay_ms).collect();
        assert_eq!(delays, [1000, 2000, 4000, 8000, 16000, 30000, 30000]);
        assert_eq!(retry_delay_ms(u32::MAX), MAX_RETRY_DELAY_MS);
    }
}
