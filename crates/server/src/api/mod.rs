mod uploads;

use axum::Router;
use axum::routing::get;

use crate::auth;
use crate::http::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(auth::me))
        .route("/uploads", get(uploads::pending))
}
