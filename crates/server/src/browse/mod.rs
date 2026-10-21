mod routes;

use axum::Router;
use axum::routing::{get, post};

use crate::http::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/volumes", get(routes::volumes))
        .route("/volumes/{volume}/entries", get(routes::entries))
        .route("/volumes/{volume}/folders", post(routes::create_folder))
}
