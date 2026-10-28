use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use shared::api::browse::{
    CreateFolder, Entry, EntryKind, Listing, Rename, Renamed, Space, VolumeInfo,
};

use crate::auth::Action;
use crate::auth::CurrentUser;
use crate::http::AppState;
use crate::http::access::{CancelOnDrop, authorize, blocking_volume};
use crate::http::error::ApiError;
use crate::storage::{self, InvalidPath};

#[derive(Deserialize)]
pub struct EntriesQuery {
    #[serde(default)]
    path: String,
}

pub async fn volumes(
    State(state): State<AppState>,
    _user: CurrentUser,
) -> Result<Json<Vec<VolumeInfo>>, ApiError> {
    let volumes = Arc::clone(&state.volumes);
    let infos = tokio::task::spawn_blocking(move || {
        volumes
            .iter()
            .map(|volume| VolumeInfo {
                name: volume.name().to_owned(),
                read_only: volume.read_only(),
                space: match volume.space() {
                    Ok(space) => Some(Space {
                        total: space.total,
                        available: space.available,
                    }),
                    Err(err) => {
                        tracing::warn!(volume = volume.name(), error = %err, "cannot read free space");
                        None
                    }
                },
            })
            .collect()
    })
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(infos))
}

pub async fn entries(
    State(state): State<AppState>,
    user: CurrentUser,
    volume: Result<Path<String>, PathRejection>,
    query: Result<Query<EntriesQuery>, QueryRejection>,
) -> Result<Json<Listing>, ApiError> {
    let (Ok(Path(volume)), Ok(Query(query))) = (volume, query) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "Invalid volume or path parameter",
        ));
    };
    let path = authorize(&state, &user, &volume, &query.path, Action::Read)?;
    tracing::debug!(user = user.sub, volume, path = path.as_str(), "listing");

    let (_guard, cancel) = CancelOnDrop::new();
    let listed = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.list_while(&path, &|| !cancel.load(Ordering::Relaxed))
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;

    let entries = listed
        .into_iter()
        .map(|meta| Entry {
            name: meta.name,
            kind: match meta.kind {
                storage::EntryKind::Dir => EntryKind::Dir,
                storage::EntryKind::File => EntryKind::File,
                storage::EntryKind::Other => EntryKind::Other,
            },
            size: meta.size,
            modified_ms: meta.modified.and_then(epoch_millis),
        })
        .collect();
    Ok(Json(Listing {
        volume,
        path: path.as_str().to_owned(),
        entries,
    }))
}

/// Creates a folder with its missing parents, an existing folder being fine; `exclusive` creates
/// just that folder and refuses an existing name.
pub async fn create_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    volume: Result<Path<String>, PathRejection>,
    body: Result<Json<CreateFolder>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let (Ok(Path(volume)), Ok(Json(CreateFolder { path, exclusive }))) = (volume, body) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON body with a path is required",
        ));
    };
    let path = authorize(&state, &user, &volume, &path, Action::Write)?;
    if path.is_root() {
        return Err(InvalidPath {
            reason: "empty path",
        }
        .into());
    }
    blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| {
            if exclusive {
                vol.create_dir(&path)
            } else {
                vol.create_dir_all(&path)
            }
        }
    })
    .await?
    .map_err(|err| {
        tracing::warn!(
            user = user.sub,
            action = "create_folder",
            volume,
            path = path.as_str(),
            error = %err,
            "create folder failed"
        );
        ApiError::from_logged_storage(&volume, err)
    })?;
    tracing::info!(
        user = user.sub,
        action = "create_folder",
        volume,
        path = path.as_str(),
        "folder created"
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rename(
    State(state): State<AppState>,
    user: CurrentUser,
    volume: Result<Path<String>, PathRejection>,
    body: Result<Json<Rename>, JsonRejection>,
) -> Result<Json<Renamed>, ApiError> {
    let (
        Ok(Path(volume)),
        Ok(Json(Rename {
            path,
            name,
            conflict,
        })),
    ) = (volume, body)
    else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON body with a path and a name is required",
        ));
    };
    let path = authorize(&state, &user, &volume, &path, Action::Write)?;
    let renamed = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.rename(&path, &name, conflict)
    })
    .await?
    .map_err(|err| {
        tracing::warn!(
            user = user.sub,
            action = "rename",
            volume,
            path = path.as_str(),
            error = %err,
            "rename failed"
        );
        ApiError::from_logged_storage(&volume, err)
    })?;
    tracing::info!(
        user = user.sub,
        action = "rename",
        volume,
        path = path.as_str(),
        final_path = renamed.as_str(),
        "entry renamed"
    );
    Ok(Json(Renamed {
        path: renamed.as_str().to_owned(),
    }))
}

pub(crate) fn epoch_millis(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|ms| -ms),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn epoch_millis_signs_by_side_of_the_epoch() {
        assert_eq!(epoch_millis(UNIX_EPOCH), Some(0));
        assert_eq!(
            epoch_millis(UNIX_EPOCH + Duration::from_millis(1500)),
            Some(1500)
        );
        assert_eq!(
            epoch_millis(UNIX_EPOCH - Duration::from_millis(1500)),
            Some(-1500)
        );
    }
}
