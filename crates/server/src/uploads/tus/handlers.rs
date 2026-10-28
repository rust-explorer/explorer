use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap};
use axum::response::{IntoResponse, Response};
use shared::api::uploads::MAX_PATCHES_PER_USER;
use shared::tus::TUS_VERSION;

use super::admit::{check_pending_count, check_space, supersede};
use super::commit::{commit, now_ms, write_and_commit};
use super::protocol::{
    Creation, EXTENSIONS, OFFSET_STREAM, TUS_EXTENSION, TUS_VERSION_HEADER, UPLOAD_LENGTH,
    UPLOAD_OFFSET, no_content, parse_creation, parse_decimal,
};
use super::staged::{forget, open_staged, remove_staged, staged_len};
use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::{authorize, blocking_volume};
use crate::http::error::ApiError;
use crate::storage::{InvalidPath, RelPath, StagedId};
use crate::uploads::UploadRecord;

pub(super) async fn discover(_user: CurrentUser) -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (TUS_VERSION_HEADER, TUS_VERSION),
            (TUS_EXTENSION, EXTENSIONS),
        ],
    )
        .into_response()
}

fn root_is_invalid(path: &RelPath) -> Result<(), ApiError> {
    if path.is_root() {
        return Err(InvalidPath {
            reason: "empty path",
        }
        .into());
    }
    Ok(())
}

pub(super) async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let creation = parse_creation(&headers)?;
    let path = authorize(
        &state,
        &user,
        &creation.volume,
        &creation.path,
        Action::Write,
    )?;
    root_is_invalid(&path)?;
    let Creation {
        volume,
        length,
        modified_ms,
        conflict,
        fingerprint,
        ..
    } = creation;

    let admission = state.uploads.admission(&volume).await;
    supersede(&state, &user.sub, &volume, &path).await?;
    check_pending_count(&state, &user.sub).await?;
    let pending = state
        .uploads
        .pending_on(&state.volumes.same_filesystem(&volume))
        .await
        .map_err(ApiError::internal)?;
    let id = StagedId::random();
    blocking_volume(&state.volumes, &volume, {
        let (id, path, volumes) = (id.clone(), path.clone(), Arc::clone(&state.volumes));
        move |vol| {
            check_space(&volumes, vol, &path, length, &pending)?;
            vol.check_upload_target(&path, conflict)
                .and_then(|()| vol.staging())
                .and_then(|staging| staging.create(&id))
                .map(drop)
                .map_err(|err| ApiError::from_storage(vol.name(), err))
        }
    })
    .await??;

    let now = now_ms();
    let record = UploadRecord {
        id: id.clone(),
        user_sub: user.sub.clone(),
        volume,
        path,
        length,
        modified_ms,
        conflict,
        fingerprint,
        final_path: None,
        created_at: now,
        updated_at: now,
    };
    if let Err(err) = state.uploads.insert(&record).await {
        if let Err(cleanup) = remove_staged(&state, &record).await {
            tracing::warn!(error = ?cleanup, "cannot remove the staged file of a failed upload");
        }
        return Err(ApiError::internal(err));
    }
    drop(admission);
    tracing::info!(
        user = user.sub,
        action = "upload",
        volume = record.volume,
        path = record.path.as_str(),
        length,
        "upload started"
    );
    if length == 0 {
        let lock = state
            .uploads
            .try_lock(&id)
            .ok_or_else(ApiError::upload_busy)?;
        let (state, sub) = (state.clone(), user.sub.clone());
        tokio::spawn(async move {
            let _lock = lock;
            commit(&state, &sub, &record).await
        })
        .await
        .map_err(ApiError::internal)??;
    }

    Ok((
        StatusCode::CREATED,
        [
            (header::LOCATION, format!("/files/uploads/{}", id.as_str())),
            (UPLOAD_OFFSET, "0".to_owned()),
        ],
    )
        .into_response())
}

pub(super) async fn status(
    State(state): State<AppState>,
    user: CurrentUser,
    id: Result<Path<String>, PathRejection>,
) -> Result<Response, ApiError> {
    let record = find(&state, &user, id).await?;
    let offset = if record.final_path.is_some() {
        record.length
    } else if let Some(len) = staged_len(&state, &record).await? {
        len
    } else if let Some(_lock) = state.uploads.try_lock(&record.id) {
        match reload(&state, &record).await? {
            Some(record) if record.final_path.is_some() => record.length,
            _ => {
                forget(&state, &record).await;
                return Err(ApiError::upload_not_found());
            }
        }
    } else {
        // The request holding the lock moved the staged file into place and is finishing.
        record.length
    };
    Ok((
        StatusCode::OK,
        [
            (UPLOAD_OFFSET, offset.to_string()),
            (UPLOAD_LENGTH, record.length.to_string()),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
    )
        .into_response())
}

pub(super) async fn append(
    State(state): State<AppState>,
    user: CurrentUser,
    id: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let streamed = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case(OFFSET_STREAM));
    if !streamed {
        return Err(ApiError::unsupported_media_type(OFFSET_STREAM));
    }
    let offset = headers
        .get(UPLOAD_OFFSET)
        .and_then(parse_decimal)
        .ok_or_else(|| ApiError::invalid_upload("Upload-Offset is required"))?;
    let record = find(&state, &user, id).await?;
    authorize(
        &state,
        &user,
        &record.volume,
        record.path.as_str(),
        Action::Write,
    )?;
    if record.final_path.is_some() {
        return committed(&record, offset);
    }

    let Some(slot) = state
        .uploads
        .try_start_patch(&user.sub, MAX_PATCHES_PER_USER)
    else {
        tracing::warn!(user = user.sub, "too many upload requests in flight");
        return Err(ApiError::too_many_patches());
    };
    let Some(lock) = state.uploads.try_lock(&record.id) else {
        return Err(ApiError::upload_busy());
    };
    // The request that held the lock may have committed it.
    let record = reload(&state, &record)
        .await?
        .ok_or_else(ApiError::upload_not_found)?;
    if record.final_path.is_some() {
        return committed(&record, offset);
    }
    let Some(staged) = open_staged(&state, &record).await? else {
        forget(&state, &record).await;
        return Err(ApiError::upload_not_found());
    };
    if offset != staged.1 {
        return Err(ApiError::offset_mismatch(staged.1));
    }

    // A task of its own, holding the lock: if the client goes away and this handler is dropped,
    // the writes still complete and the commit still gets recorded before the lock is released.
    tokio::spawn(async move {
        let _slot = slot;
        write_and_commit(state, user.sub, record, staged, body, lock).await
    })
    .await
    .map_err(ApiError::internal)?
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let record = find(&state, &user, id).await?;
    let Some(_lock) = state.uploads.lock_or_request_cancel(&record.id) else {
        return Err(ApiError::upload_busy());
    };
    let Some(record) = reload(&state, &record).await? else {
        return Err(ApiError::upload_not_found());
    };
    if record.final_path.is_none() {
        remove_staged(&state, &record).await?;
    }
    state
        .uploads
        .delete(&record.id)
        .await
        .map_err(ApiError::internal)?;
    // The cancel lost the race with the last chunk: the client must not report it cancelled.
    if let Some(final_path) = &record.final_path {
        return Err(ApiError::upload_committed(final_path.as_str()));
    }
    tracing::info!(
        user = user.sub,
        action = "upload",
        volume = record.volume,
        path = record.path.as_str(),
        "upload cancelled"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// A committed upload only accepts an empty PATCH at its end, as a retry of the last one.
fn committed(record: &UploadRecord, offset: u64) -> Result<Response, ApiError> {
    if offset == record.length {
        Ok(no_content(offset))
    } else {
        Err(ApiError::offset_mismatch(record.length))
    }
}

/// Another user's upload, or an id that is not one, is not found.
async fn find(
    state: &AppState,
    user: &CurrentUser,
    id: Result<Path<String>, PathRejection>,
) -> Result<UploadRecord, ApiError> {
    let id = id
        .ok()
        .and_then(|Path(id)| StagedId::parse(&id))
        .ok_or_else(ApiError::upload_not_found)?;
    state
        .uploads
        .get(&id, &user.sub)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::upload_not_found)
}

async fn reload(state: &AppState, record: &UploadRecord) -> Result<Option<UploadRecord>, ApiError> {
    state
        .uploads
        .get(&record.id, &record.user_sub)
        .await
        .map_err(ApiError::internal)
}
