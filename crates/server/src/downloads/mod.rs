mod channel;
mod file;
mod selection;
mod state;
mod zip;

use axum::Router;
use axum::routing::{get, post};

use crate::http::AppState;

pub use file::{attachment_headers, content_disposition};
pub use state::{Downloads, MAX_SELECTIONS_PER_USER, SELECTION_TTL, ZIP_CONCURRENCY};
pub use zip::SKIPPED_LIST;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/download", get(file::download))
        .route("/zip", get(zip::zip).head(zip::head))
        .route("/zip/selections", post(selection::create))
}
