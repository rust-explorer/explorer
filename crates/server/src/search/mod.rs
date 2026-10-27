mod routes;

use std::time::Duration;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
pub const MIN_QUERY_CHARS: usize = 2;

pub fn routes() -> Router<AppState> {
    Router::new().route("/volumes/{volume}/search", get(routes::search))
}
