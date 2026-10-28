mod registry;
mod routes;
mod run;
mod transfer;
mod usage;

use std::time::Duration;

use axum::Router;
use axum::routing::{delete, get};

use crate::http::AppState;

pub use registry::{Progress, Tasks};
pub use run::TaskError;

/// How many tasks run at once across all users; the others wait, queued.
pub const MAX_RUNNING: usize = 2;
/// A finished task stays listed this long unless dismissed.
pub const FINISHED_TTL: Duration = Duration::from_secs(3600);
/// Finished usage scans whose result a user keeps; older ones answer as expired.
pub const MAX_USAGE_TREES: usize = 4;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/tasks", get(routes::list).post(routes::create))
        .route("/tasks/{id}", delete(routes::cancel))
        .route("/tasks/{id}/usage", get(usage::view))
}
