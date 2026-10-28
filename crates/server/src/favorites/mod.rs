mod routes;
mod store;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

pub use store::{Added, Favorites};

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/favorites",
        get(routes::list).put(routes::add).delete(routes::remove),
    )
}
