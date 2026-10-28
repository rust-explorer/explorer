use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use shared::api::conflict::ConflictPolicy;
use shared::api::tasks::{Location, TaskInfo, TaskSpec};

use super::registry::Stopped;
use super::transfer::{TransferKind, TransferTask};
use super::{Progress, TaskError};
use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::{authorize, blocking_volume};
use crate::http::error::ApiError;
use crate::http::request_id::request_id;
use crate::storage::{RelPath, Volumes};

type Op = Box<dyn FnOnce(&Progress) -> Result<(), TaskError> + Send>;

pub async fn list(State(state): State<AppState>, user: CurrentUser) -> Json<Vec<TaskInfo>> {
    Json(state.tasks.list(&user.sub))
}

/// Checks the task, then queues it; whatever fails from then on is reported by the task.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    body: Result<Json<TaskSpec>, JsonRejection>,
) -> Result<(StatusCode, Json<TaskInfo>), ApiError> {
    let Ok(Json(spec)) = body else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON task is required",
        ));
    };
    let op: Op = match &spec {
        TaskSpec::Delete { source } => {
            let path = authorize(&state, &user, &source.volume, &source.path, Action::Write)?;
            blocking_volume(&state.volumes, &source.volume, {
                let path = path.clone();
                move |vol| vol.check_deletable(&path)
            })
            .await?
            .map_err(|err| {
                tracing::warn!(
                    user = user.sub,
                    action = "delete",
                    volume = source.volume,
                    path = path.as_str(),
                    error = %err,
                    "delete refused"
                );
                ApiError::from_logged_storage(&source.volume, err)
            })?;
            Box::new(delete_op(
                Arc::clone(&state.volumes),
                user.sub.clone(),
                source.volume.clone(),
                path,
            ))
        }
        TaskSpec::Copy {
            source,
            dest_dir,
            conflict,
        } => {
            transfer_op(
                &state,
                &user,
                TransferKind::Copy,
                source,
                dest_dir,
                *conflict,
            )
            .await?
        }
        TaskSpec::Move {
            source,
            dest_dir,
            conflict,
        } => {
            transfer_op(
                &state,
                &user,
                TransferKind::Move,
                source,
                dest_dir,
                *conflict,
            )
            .await?
        }
    };
    let request_id = Some(request_id(&headers).to_owned()).filter(|id| !id.is_empty());
    let info = state.tasks.submit(&user.sub, spec, request_id, op);
    Ok((StatusCode::ACCEPTED, Json(info)))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let action = match state.tasks.cancel_or_dismiss(&user.sub, &id) {
        Some(Stopped::Cancelled) => "cancel",
        Some(Stopped::Dismissed) => "dismiss",
        None => return Err(ApiError::task_not_found()),
    };
    tracing::info!(user = user.sub, action, id, "task stopped");
    Ok(StatusCode::NO_CONTENT)
}

/// Copy needs to read the source, move to write it; both write the destination.
async fn transfer_op(
    state: &AppState,
    user: &CurrentUser,
    kind: TransferKind,
    source: &Location,
    dest_dir: &Location,
    conflict: ConflictPolicy,
) -> Result<Op, ApiError> {
    let source_access = match kind {
        TransferKind::Copy => Action::Read,
        TransferKind::Move => Action::Write,
    };
    let task = TransferTask {
        kind,
        path: authorize(state, user, &source.volume, &source.path, source_access)?,
        dest_dir: authorize(state, user, &dest_dir.volume, &dest_dir.path, Action::Write)?,
        source: source.volume.clone(),
        dest: dest_dir.volume.clone(),
        conflict,
    };
    task.check(&state.volumes, &user.sub).await?;
    Ok(Box::new(
        task.op(Arc::clone(&state.volumes), user.sub.clone()),
    ))
}

fn delete_op(
    volumes: Arc<Volumes>,
    user: String,
    volume: String,
    path: RelPath,
) -> impl FnOnce(&Progress) -> Result<(), TaskError> + Send + 'static {
    move |progress| {
        progress.set_files_total(1);
        let vol = volumes
            .get(&volume)
            .ok_or_else(|| ApiError::volume_not_found(&volume))?;
        if let Err(err) = vol.delete(&path) {
            tracing::warn!(
                user,
                action = "delete",
                volume,
                path = path.as_str(),
                error = %err,
                "delete failed"
            );
            return Err(ApiError::from_logged_storage(&volume, err).into());
        }
        progress.add_file();
        tracing::info!(
            user,
            action = "delete",
            volume,
            path = path.as_str(),
            "entry deleted"
        );
        Ok(())
    }
}
