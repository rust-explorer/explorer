mod download;
mod zip;

use axum::Router;
use axum::routing::get;

use crate::app::AppState;
use crate::uploads;

pub use download::{attachment_headers, content_disposition};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/download", get(download::download))
        .route("/zip", get(zip::zip).head(zip::head))
        .nest("/uploads", uploads::tus_routes())
}
