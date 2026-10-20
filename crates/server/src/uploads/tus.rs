use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{head, options};
use rustix::io::Errno;
use shared::api::ConflictPolicy;
use shared::tus::{TUS_VERSION, decode_metadata};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

use super::UploadRecord;
use crate::access::{authorize, blocking_volume};
use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::authz::Action;
use crate::error::ApiError;
use crate::storage::{InvalidPath, RelPath, StorageError, UploadId};

/// A PATCH whose body sends nothing for this long is treated as a disconnect.
pub const PATCH_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

const OFFSET_STREAM: &str = "application/offset+octet-stream";
const EXTENSIONS: &str = "creation,termination";

const TUS_RESUMABLE: HeaderName = HeaderName::from_static("tus-resumable");
const TUS_VERSION_HEADER: HeaderName = HeaderName::from_static("tus-version");
const TUS_EXTENSION: HeaderName = HeaderName::from_static("tus-extension");
const UPLOAD_LENGTH: HeaderName = HeaderName::from_static("upload-length");
const UPLOAD_OFFSET: HeaderName = HeaderName::from_static("upload-offset");
const UPLOAD_METADATA: HeaderName = HeaderName::from_static("upload-metadata");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", options(discover).post(create))
        .route("/{id}", head(status).patch(append).delete(cancel))
        .fallback(not_found)
        .layer(middleware::from_fn(tus_resumable))
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}

/// Requires `Tus-Resumable` on every request but `OPTIONS` and sends it on every response.
async fn tus_resumable(req: Request, next: Next) -> Response {
    let supported = req.method() == Method::OPTIONS
        || req
            .headers()
            .get(&TUS_RESUMABLE)
            .is_some_and(|v| v.as_bytes() == TUS_VERSION.as_bytes());
    let mut res = if supported {
        next.run(req).await
    } else {
        let mut res = ApiError::unsupported_tus_version().into_response();
        res.headers_mut()
            .insert(TUS_VERSION_HEADER, HeaderValue::from_static(TUS_VERSION));
        res
    };
    res.headers_mut()
        .insert(TUS_RESUMABLE, HeaderValue::from_static(TUS_VERSION));
    res
}

async fn discover(_user: CurrentUser) -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (TUS_VERSION_HEADER, TUS_VERSION),
            (TUS_EXTENSION, EXTENSIONS),
        ],
    )
        .into_response()
}

struct Creation {
    volume: String,
    path: String,
    length: u64,
    modified_ms: Option<i64>,
    conflict: ConflictPolicy,
}

fn parse_creation(headers: &HeaderMap) -> Result<Creation, ApiError> {
    let length = headers
        .get(UPLOAD_LENGTH)
        .and_then(parse_decimal)
        .ok_or_else(|| ApiError::invalid_upload("Upload-Length is required"))?;
    let metadata = headers
        .get(UPLOAD_METADATA)
        .and_then(|v| v.to_str().ok())
        .and_then(decode_metadata)
        .ok_or_else(|| ApiError::invalid_upload("Upload-Metadata is missing or malformed"))?;
    let value = |key: &str| {
        metadata
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let required = |key: &str| {
        value(key)
            .map(str::to_owned)
            .ok_or_else(|| ApiError::invalid_upload(format!("Upload-Metadata lacks {key}")))
    };
    let modified_ms = value("modified")
        .map(str::parse)
        .transpose()
        .map_err(|_| ApiError::invalid_upload("modified must be integer milliseconds"))?;
    let conflict = value("conflict")
        .map(|v| {
            ConflictPolicy::parse(v).ok_or_else(|| {
                ApiError::invalid_upload("conflict must be fail, replace or keep_both")
            })
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Creation {
        volume: required("volume")?,
        path: required("path")?,
        length,
        modified_ms,
        conflict,
    })
}

fn parse_decimal(value: &HeaderValue) -> Option<u64> {
    let value = value.to_str().ok()?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
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

async fn create(
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
        ..
    } = creation;

    let id = UploadId::random();
    blocking_volume(&state.volumes, &volume, {
        let (id, path) = (id.clone(), path.clone());
        move |vol| {
            match vol.space() {
                Ok(space) if length > space.available => {
                    tracing::warn!(
                        volume = vol.name(),
                        path = path.as_str(),
                        length,
                        available = space.available,
                        "upload larger than the free space"
                    );
                    return Err(ApiError::insufficient_storage(vol.name(), path.as_str()));
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(volume = vol.name(), error = %err, "cannot read free space");
                }
            }
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
    tracing::info!(
        user = user.sub,
        action = "upload",
        volume = record.volume,
        path = record.path.as_str(),
        length,
        "upload started"
    );
    if length == 0 {
        commit(&state, &user, &record).await?;
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

async fn status(
    State(state): State<AppState>,
    user: CurrentUser,
    id: Result<Path<String>, PathRejection>,
) -> Result<Response, ApiError> {
    let record = find(&state, &user, id).await?;
    let offset = if record.final_path.is_some() {
        record.length
    } else if let Some(len) = staged_len(&state, &record).await? {
        len
    } else {
        forget(&state, &record).await;
        return Err(ApiError::upload_not_found());
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

async fn append(
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

    let Some(_lock) = state.uploads.try_lock(&record.id) else {
        return Err(ApiError::upload_busy());
    };
    // The request that held the lock may have committed it.
    let record = state
        .uploads
        .get(&record.id, &user.sub)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::upload_not_found)?;
    if record.final_path.is_some() {
        return committed(&record, offset);
    }
    let Some((file, staged)) = open_staged(&state, &record).await? else {
        forget(&state, &record).await;
        return Err(ApiError::upload_not_found());
    };
    if offset != staged {
        return Err(ApiError::offset_mismatch(staged));
    }

    let (offset, too_large) = receive(file, body, staged, record.length)
        .await
        .map_err(|err| write_error(&record, &err))?;
    state
        .uploads
        .touch(&record.id, now_ms())
        .await
        .map_err(ApiError::internal)?;
    if too_large {
        tracing::warn!(
            user = user.sub,
            volume = record.volume,
            path = record.path.as_str(),
            length = record.length,
            "upload body longer than Upload-Length"
        );
        return Err(ApiError::upload_too_large(record.length));
    }
    if offset == record.length {
        commit(&state, &user, &record).await?;
    }
    Ok(no_content(offset))
}

async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    id: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let record = find(&state, &user, id).await?;
    if record.final_path.is_none() {
        remove_staged(&state, &record).await?;
    }
    state
        .uploads
        .delete(&record.id)
        .await
        .map_err(ApiError::internal)?;
    if record.final_path.is_none() {
        tracing::info!(
            user = user.sub,
            action = "upload",
            volume = record.volume,
            path = record.path.as_str(),
            "upload cancelled"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

fn no_content(offset: u64) -> Response {
    (
        StatusCode::NO_CONTENT,
        [(UPLOAD_OFFSET, offset.to_string())],
    )
        .into_response()
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
        .and_then(|Path(id)| UploadId::parse(&id))
        .ok_or_else(ApiError::upload_not_found)?;
    state
        .uploads
        .get(&id, &user.sub)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::upload_not_found)
}

/// The length of the staged file; `None` when it, its staging directory or its volume is gone.
pub(crate) async fn staged_len(
    state: &AppState,
    record: &UploadRecord,
) -> Result<Option<u64>, ApiError> {
    if state.volumes.get(&record.volume).is_none() {
        return Ok(None);
    }
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        match vol.existing_staging()? {
            Some(staging) => staging.len(&id),
            None => Ok(None),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

async fn open_staged(
    state: &AppState,
    record: &UploadRecord,
) -> Result<Option<(std::fs::File, u64)>, ApiError> {
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        let Some(staging) = vol.existing_staging()? else {
            return Ok(None);
        };
        match staging.open_append(&id) {
            Ok(opened) => Ok(Some(opened)),
            Err(StorageError::NotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

async fn remove_staged(state: &AppState, record: &UploadRecord) -> Result<(), ApiError> {
    if state.volumes.get(&record.volume).is_none() {
        return Ok(());
    }
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        match vol.existing_staging()? {
            Some(staging) => staging.remove(&id),
            None => Ok(()),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

/// Drops a pending record whose staged file is gone.
async fn forget(state: &AppState, record: &UploadRecord) {
    tracing::warn!(
        volume = record.volume,
        path = record.path.as_str(),
        "staged upload file is missing; dropping the upload"
    );
    if let Err(err) = state.uploads.delete(&record.id).await {
        tracing::warn!(error = %err, "cannot delete upload record");
    }
}

/// Appends the body to the staged file until it ends, fails or idles, keeping at most
/// `length - offset` bytes. Returns the new offset and whether the body had more than that.
async fn receive(
    file: std::fs::File,
    body: Body,
    offset: u64,
    length: u64,
) -> io::Result<(u64, bool)> {
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
    let mut offset = offset;
    let mut too_large = false;
    loop {
        let chunk = match tokio::time::timeout(PATCH_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(None) => break,
            Ok(Some(Err(err))) => {
                tracing::debug!(error = %err, offset, "upload body ended early");
                break;
            }
            Err(_) => {
                tracing::debug!(offset, "upload body idle, closing");
                break;
            }
        };
        let room = usize::try_from(length - offset).unwrap_or(usize::MAX);
        let kept = &chunk[..chunk.len().min(room)];
        file.write_all(kept).await?;
        offset += kept.len() as u64;
        if kept.len() < chunk.len() {
            too_large = true;
            break;
        }
    }
    file.flush().await?;
    file.sync_data().await?;
    Ok((offset, too_large))
}

fn write_error(record: &UploadRecord, err: &io::Error) -> ApiError {
    tracing::warn!(
        volume = record.volume,
        path = record.path.as_str(),
        error = %err,
        "cannot write upload data"
    );
    let no_space = err.kind() == io::ErrorKind::StorageFull
        || [Errno::NOSPC, Errno::DQUOT]
            .iter()
            .any(|errno| err.raw_os_error() == Some(errno.raw_os_error()));
    if no_space {
        return ApiError::insufficient_storage(&record.volume, record.path.as_str());
    }
    ApiError::internal(err)
}

/// Moves the complete staged file into place and marks the upload done. On failure the staged
/// file stays, so an empty PATCH at the end retries.
async fn commit(
    state: &AppState,
    user: &CurrentUser,
    record: &UploadRecord,
) -> Result<(), ApiError> {
    let final_path = blocking_volume(&state.volumes, &record.volume, {
        let id = record.id.clone();
        let path = record.path.clone();
        let conflict = record.conflict;
        let modified = record.modified_ms.and_then(system_time);
        move |vol| {
            let staging = vol
                .existing_staging()?
                .ok_or_else(|| StorageError::NotFound(RelPath::root()))?;
            vol.commit_upload(&staging, &id, &path, conflict, modified)
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))?;
    state
        .uploads
        .mark_done(&record.id, &final_path, now_ms())
        .await
        .map_err(ApiError::internal)?;
    tracing::info!(
        user = user.sub,
        action = "upload",
        volume = record.volume,
        path = record.path.as_str(),
        bytes = record.length,
        final_path = final_path.as_str(),
        "upload completed"
    );
    Ok(())
}

fn system_time(ms: i64) -> Option<SystemTime> {
    let offset = Duration::from_millis(ms.unsigned_abs());
    if ms >= 0 {
        UNIX_EPOCH.checked_add(offset)
    } else {
        UNIX_EPOCH.checked_sub(offset)
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_headers() {
        let parse = |s: &str| parse_decimal(&HeaderValue::from_str(s).unwrap());
        assert_eq!(parse("0"), Some(0));
        assert_eq!(parse("1048576"), Some(1_048_576));
        for bad in ["", "-1", "+1", " 1", "1.0", "0x10", "99999999999999999999"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn system_time_from_millis() {
        assert_eq!(system_time(0), Some(UNIX_EPOCH));
        assert_eq!(
            system_time(1500),
            Some(UNIX_EPOCH + Duration::from_millis(1500))
        );
        assert_eq!(
            system_time(-1500),
            Some(UNIX_EPOCH - Duration::from_millis(1500))
        );
    }
}
