mod file;
mod zip;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

pub use file::{attachment_headers, content_disposition};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/download", get(file::download))
        .route("/zip", get(zip::zip).head(zip::head))
}
