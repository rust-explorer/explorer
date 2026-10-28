mod browse;
mod folders;
mod uploads;

use axum::Router;
use axum::routing::{get, post};

use crate::app::AppState;
use crate::auth;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(auth::me))
        .route("/volumes", get(browse::volumes))
        .route("/volumes/{volume}/entries", get(browse::entries))
        .route("/volumes/{volume}/folders", post(folders::create))
        .route("/uploads", get(uploads::pending))
}
