mod commit;
mod handlers;
mod protocol;
mod staged;

use axum::Router;
use axum::middleware;
use axum::routing::{head, options};

use crate::http::AppState;
use crate::http::error::ApiError;

pub use commit::PATCH_IDLE_TIMEOUT;
pub use protocol::tag_tus_responses;
pub(crate) use staged::staged_len;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", options(handlers::discover).post(handlers::create))
        .route(
            "/{id}",
            head(handlers::status)
                .patch(handlers::append)
                .delete(handlers::cancel),
        )
        .fallback(not_found)
        .layer(middleware::from_fn(protocol::require_tus_version))
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}
