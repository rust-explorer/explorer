use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use shared::api::{Entry, EntryKind, Listing, Space, VolumeInfo};

use super::blocking_volume;
use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::authz::{Action, Policy};
use crate::error::ApiError;
use crate::storage::{self, RelPath};

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
    if state.volumes.get(&volume).is_none() {
        return Err(ApiError::volume_not_found(&volume));
    }
    Policy.check(&user, &volume, Action::Read)?;
    let path = RelPath::parse(&query.path)?;
    tracing::debug!(user = user.sub, volume, path = path.as_str(), "listing");

    let listed = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.list(&path)
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

fn epoch_millis(time: SystemTime) -> Option<i64> {
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
