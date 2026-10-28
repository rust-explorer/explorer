mod routes;

use std::time::Duration;

use axum::Router;
use axum::routing::get;

use crate::http::AppState;

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Searches walking at once, across all users; more wait for one to end.
pub const SEARCH_CONCURRENCY: usize = 4;

pub fn routes() -> Router<AppState> {
    Router::new().route("/volumes/{volume}/search", get(routes::search))
}
