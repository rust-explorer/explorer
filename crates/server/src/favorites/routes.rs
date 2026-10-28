use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use shared::api::favorites::Favorite;
use shared::api::tasks::Location;

use super::Added;
use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::{VolumeQuery, authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::{EntryKind, RelPath};

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<Favorite>>, ApiError> {
    let rows = state
        .favorites
        .list(&user.sub)
        .await
        .map_err(ApiError::internal)?;
    let volumes = std::sync::Arc::clone(&state.volumes);
    let favorites = tokio::task::spawn_blocking(move || {
        rows.into_iter()
            .map(|(volume, path)| {
                let exists = volumes.get(&volume).is_some_and(|vol| {
                    RelPath::parse(&path)
                        .ok()
                        .and_then(|path| vol.stat(&path).ok())
                        .is_some_and(|meta| meta.kind == EntryKind::Dir)
                });
                Favorite {
                    volume,
                    path,
                    exists,
                }
            })
            .collect()
    })
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(favorites))
}

pub async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    body: Result<Json<Location>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Ok(Json(location)) = body else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON location is required",
        ));
    };
    let path = authorize(
        &state,
        &user,
        &location.volume,
        &location.path,
        Action::Read,
    )?;
    let meta = blocking_volume(&state.volumes, &location.volume, {
        let path = path.clone();
        move |vol| vol.stat(&path)
    })
    .await?
    .map_err(|err| ApiError::from_storage(&location.volume, err))?;
    if meta.kind != EntryKind::Dir {
        return Err(ApiError::from_storage(
            &location.volume,
            crate::storage::StorageError::NotADirectory(path),
        ));
    }
    let added = state
        .favorites
        .add(&user.sub, &location.volume, path.as_str())
        .await
        .map_err(ApiError::internal)?;
    match added {
        Added::Added => tracing::info!(
            user = user.sub,
            action = "favorite_add",
            volume = location.volume,
            path = path.as_str(),
            "favorite added"
        ),
        Added::AlreadyThere => {}
        Added::Full => return Err(ApiError::favorites_full()),
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    query: Result<Query<VolumeQuery>, QueryRejection>,
) -> Result<StatusCode, ApiError> {
    let Ok(Query(query)) = query else {
        return Err(invalid_query());
    };
    // Not authorized against the volume, which may be gone: only the user's own row goes.
    let path = RelPath::parse(&query.path)?;
    let removed = state
        .favorites
        .remove(&user.sub, &query.volume, path.as_str())
        .await
        .map_err(ApiError::internal)?;
    if removed {
        tracing::info!(
            user = user.sub,
            action = "favorite_remove",
            volume = query.volume,
            path = path.as_str(),
            "favorite removed"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}
