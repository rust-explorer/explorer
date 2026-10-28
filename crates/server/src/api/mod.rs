mod browse;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::app::AppState;
use crate::auth;
use crate::error::ApiError;
use crate::storage::{self, Volumes};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(auth::me))
        .route("/volumes", get(browse::volumes))
        .route("/volumes/{volume}/entries", get(browse::entries))
}

/// Runs a blocking storage call on `volume`, which must exist.
pub(crate) async fn blocking_volume<T: Send + 'static>(
    volumes: &Arc<Volumes>,
    volume: &str,
    op: impl FnOnce(&storage::Volume) -> T + Send + 'static,
) -> Result<T, ApiError> {
    let volumes = Arc::clone(volumes);
    let name = volume.to_owned();
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        volumes
            .get(&name)
            .map(op)
            .ok_or_else(|| ApiError::volume_not_found(&name))
    })
    .await
    .map_err(ApiError::internal)?
}
