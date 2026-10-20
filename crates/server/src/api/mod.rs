mod browse;

use axum::Router;
use axum::routing::get;

use crate::app::AppState;
use crate::auth;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(auth::me))
        .route("/volumes", get(browse::volumes))
        .route("/volumes/{volume}/entries", get(browse::entries))
}
