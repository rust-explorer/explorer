mod download;

use axum::Router;
use axum::routing::get;

use crate::app::AppState;

pub use download::{attachment_headers, content_disposition};

pub fn routes() -> Router<AppState> {
    Router::new().route("/download", get(download::download))
}
