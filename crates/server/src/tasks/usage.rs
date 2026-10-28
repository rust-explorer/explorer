use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use shared::api::usage::UsageView;

use super::{Progress, TaskError};
use crate::auth::{Action, CurrentUser};
use crate::browse::epoch_millis;
use crate::http::AppState;
use crate::http::access::{authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::{self, EntryKind, RelPath, StorageError, Volumes};

/// Checks that the user may read `path` and that it is a folder.
pub(super) async fn check(
    state: &AppState,
    user: &CurrentUser,
    volume: &str,
    raw_path: &str,
) -> Result<RelPath, ApiError> {
    let path = authorize(state, user, volume, raw_path, Action::Read)?;
    let meta = blocking_volume(&state.volumes, volume, {
        let path = path.clone();
        move |vol| vol.stat(&path)
    })
    .await?
    .map_err(|err| ApiError::from_storage(volume, err))?;
    if meta.kind != EntryKind::Dir {
        return Err(ApiError::from_storage(
            volume,
            StorageError::NotADirectory(path),
        ));
    }
    Ok(path)
}

pub(super) fn op(
    volumes: Arc<Volumes>,
    user: String,
    volume: String,
    path: RelPath,
) -> impl FnOnce(&Progress) -> Result<(), TaskError> + Send + 'static {
    move |progress| {
        let vol = volumes
            .get(&volume)
            .ok_or_else(|| ApiError::volume_not_found(&volume))?;
        let started = Instant::now();
        let action = "usage";
        match storage::usage(vol, &path, progress) {
            Ok(tree) => {
                let (bytes, files, dirs) = tree.totals();
                let (_, _, skipped) = progress.counts();
                tracing::info!(
                    user,
                    action,
                    volume,
                    path = path.as_str(),
                    files,
                    dirs,
                    bytes,
                    skipped,
                    elapsed_ms = started.elapsed().as_millis(),
                    "usage"
                );
                progress.set_usage(tree);
                Ok(())
            }
            Err(StorageError::Cancelled) => {
                tracing::info!(
                    user,
                    action,
                    volume,
                    path = path.as_str(),
                    "usage cancelled"
                );
                Err(TaskError::Cancelled)
            }
            Err(err) => {
                tracing::warn!(
                    user,
                    action,
                    volume,
                    path = path.as_str(),
                    error = %err,
                    "usage failed"
                );
                Err(ApiError::from_logged_storage(&volume, err).into())
            }
        }
    }
}

#[derive(Deserialize)]
pub struct ViewQuery {
    path: Option<String>,
}

/// One folder of a finished scan; the scanned folder itself by default. Access is checked again:
/// it may have changed since the scan.
pub async fn view(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    query: Result<Query<ViewQuery>, QueryRejection>,
) -> Result<Json<UsageView>, ApiError> {
    let Ok(Query(query)) = query else {
        return Err(invalid_query());
    };
    let (volume, tree) = state
        .tasks
        .usage(&user.sub, &id)
        .ok_or_else(ApiError::task_not_found)?;
    authorize(&state, &user, &volume, tree.root().as_str(), Action::Read)?;
    let path = match query.path {
        Some(raw) => RelPath::parse(&raw)?,
        None => tree.root().clone(),
    };
    let view = tree
        .view(&path)
        .ok_or_else(|| ApiError::from_logged_storage(&volume, StorageError::NotFound(path)))?;
    Ok(Json(UsageView {
        volume,
        root: tree.root().as_str().to_owned(),
        path: view.path.as_str().to_owned(),
        scanned_at_ms: epoch_millis(tree.scanned_at()).unwrap_or_default(),
        bytes: view.bytes,
        files: view.files,
        dirs: view.dirs,
        incomplete: view.incomplete,
        collapsed: view.collapsed,
        items: view.items,
    }))
}
