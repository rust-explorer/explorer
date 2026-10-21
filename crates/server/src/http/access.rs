use std::sync::Arc;

use axum::http::StatusCode;
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::auth::{Action, Policy};
use crate::http::AppState;
use crate::http::error::ApiError;
use crate::storage::{self, RelPath, Volumes};

/// Query of the `/files/*` endpoints.
#[derive(Deserialize)]
pub struct VolumeQuery {
    pub volume: String,
    pub path: String,
}

/// Looks the volume up, checks the policy, rejects writes to a read-only volume, then parses
/// the path.
pub fn authorize(
    state: &AppState,
    user: &CurrentUser,
    volume: &str,
    raw_path: &str,
    action: Action,
) -> Result<RelPath, ApiError> {
    let vol = state
        .volumes
        .get(volume)
        .ok_or_else(|| ApiError::volume_not_found(volume))?;
    Policy.check(user, volume, action)?;
    if action == Action::Write && vol.read_only() {
        return Err(ApiError::read_only_volume(volume));
    }
    Ok(RelPath::parse(raw_path)?)
}

#[must_use]
pub fn invalid_query() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_path",
        "The volume and path parameters are required",
    )
}

/// Runs a blocking storage call on `volume`, which must exist.
pub async fn blocking_volume<T: Send + 'static>(
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
