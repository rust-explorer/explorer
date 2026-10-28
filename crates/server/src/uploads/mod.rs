mod cleanup;
mod routes;
mod store;
mod tus;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

pub use cleanup::{SweepReport, spawn_sweeper, sweep};
pub use store::{PatchSlot, UPLOAD_TTL, UploadLock, UploadRecord, Uploads};
pub use tus::PATCH_IDLE_TIMEOUT;
pub(crate) use tus::{routes as tus_routes, tag_tus_responses};

pub fn routes() -> Router<AppState> {
    Router::new().route("/uploads", get(routes::pending))
}
