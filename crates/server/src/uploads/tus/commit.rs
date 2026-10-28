use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::response::Response;
use rustix::io::Errno;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

use super::protocol::no_content;
use crate::http::AppState;
use crate::http::access::blocking_volume;
use crate::http::error::ApiError;
use crate::storage::{RelPath, StorageError};
use crate::uploads::{UploadLock, UploadRecord};

/// A PATCH whose body sends nothing for this long is treated as a disconnect.
pub const PATCH_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn write_and_commit(
    state: AppState,
    user_sub: String,
    record: UploadRecord,
    staged: (std::fs::File, u64),
    body: Body,
    _lock: UploadLock,
) -> Result<Response, ApiError> {
    let (offset, too_large) = receive(staged, body, record.length)
        .await
        .map_err(|err| write_error(&record, &err))?;
    state
        .uploads
        .touch(&record.id, now_ms())
        .await
        .map_err(ApiError::internal)?;
    if too_large {
        tracing::warn!(
            user = user_sub,
            volume = record.volume,
            path = record.path.as_str(),
            length = record.length,
            "upload body longer than Upload-Length"
        );
        return Err(ApiError::upload_too_large(record.length));
    }
    if offset == record.length {
        commit(&state, &user_sub, &record).await?;
    }
    Ok(no_content(offset))
}

/// Appends the body to the staged file, opened at its end, until the body ends, fails or idles,
/// keeping at most `length - offset` bytes. Returns the new offset and whether the body had more
/// than that.
async fn receive(
    (file, mut offset): (std::fs::File, u64),
    body: Body,
    length: u64,
) -> io::Result<(u64, bool)> {
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
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
        let room = usize::try_from(length.saturating_sub(offset)).unwrap_or(usize::MAX);
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

/// Moves the complete staged file into place and marks the upload done; the caller holds the
/// lock. On failure the staged file stays, so an empty PATCH at the end retries. Once the file is
/// in place the upload has succeeded, even if recording that fails.
pub(super) async fn commit(
    state: &AppState,
    user_sub: &str,
    record: &UploadRecord,
) -> Result<(), ApiError> {
    let final_path = blocking_volume(&state.volumes, &record.volume, {
        let id = record.id.clone();
        let path = record.path.clone();
        let length = record.length;
        let conflict = record.conflict;
        let modified = record.modified_ms.and_then(system_time);
        move |vol| {
            let staging = vol
                .existing_staging()?
                .ok_or_else(|| StorageError::NotFound(RelPath::root()))?;
            let staged = staging.len(&id)?;
            if staged != Some(length) {
                return Err(StorageError::Io {
                    path: path.clone(),
                    source: io::Error::other(format!(
                        "staged file has {staged:?} bytes instead of {length}; not committing"
                    )),
                });
            }
            vol.commit_staged(&staging, &id, &path, conflict, modified)
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))?;
    let mut marked = state
        .uploads
        .mark_done(&record.id, &final_path, now_ms())
        .await;
    if marked.is_err() {
        marked = state
            .uploads
            .mark_done(&record.id, &final_path, now_ms())
            .await;
    }
    if let Err(err) = marked {
        // The sweep removes the record once it expires; the file is in place either way.
        tracing::error!(
            error = %err,
            volume = record.volume,
            path = record.path.as_str(),
            final_path = final_path.as_str(),
            "upload committed but not recorded as done"
        );
    }
    tracing::info!(
        user = user_sub,
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
