use std::fs::File;
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::Response;
use shared::api::thumbnails::has_thumbnail;

use super::{MAX_SOURCE_BYTES, Thumbnail, Thumbnails, cache_key, render};
use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::{VolumeQuery, authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::{EntryKind, RelPath, StorageError, Volumes};

const CACHE_CONTROL: &str = "private, max-age=604800";

/// `GET /files/thumbnail?volume=&path=&v=`; `v` only busts the browser cache.
pub async fn thumbnail(
    State(state): State<AppState>,
    user: CurrentUser,
    query: Result<Query<VolumeQuery>, QueryRejection>,
    request_headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Ok(Query(VolumeQuery { volume, path })) = query else {
        return Err(invalid_query());
    };
    let path = authorize(&state, &user, &volume, &path, Action::Read)?;
    if !path.file_name().is_some_and(has_thumbnail) {
        return Err(ApiError::thumbnail_unavailable());
    }

    // Only a stat: the source is opened once a render permit is granted, so requests queued
    // behind the permits hold no descriptor.
    let meta = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.stat(&path)
    })
    .await?
    .map_err(|err| unreadable(&volume, &path, err))?;
    if meta.kind != EntryKind::File {
        return Err(ApiError::thumbnail_unavailable());
    }
    let size = meta.size.unwrap_or_default();
    if size > MAX_SOURCE_BYTES {
        return Err(ApiError::thumbnail_unavailable());
    }

    let key = cache_key(&volume, path.as_str(), size, meta.modified);
    let etag = HeaderValue::from_str(&format!("\"{key}\"")).map_err(ApiError::internal)?;
    if request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| etag_matches(value, &key))
    {
        tracing::debug!(
            user = user.sub,
            volume,
            path = path.as_str(),
            "thumbnail not modified"
        );
        let mut res = Response::new(Body::empty());
        *res.status_mut() = StatusCode::NOT_MODIFIED;
        cache_headers(res.headers_mut(), etag);
        return Ok(res);
    }

    let cached = lookup(&state.thumbnails, &key).await?;
    let hit = cached.is_some();
    let thumbnail = if let Some(thumbnail) = cached {
        thumbnail
    } else {
        let thumbnails = Arc::clone(&state.thumbnails);
        let source = Source {
            volumes: Arc::clone(&state.volumes),
            volume: volume.clone(),
            path: path.clone(),
            size,
            modified: meta.modified,
        };
        bounded(&state.thumbnails, move || {
            generate(&thumbnails, &key, &source)
        })
        .await?
        .map_err(|err| unreadable(&volume, &path, err))?
        .ok_or_else(ApiError::thumbnail_unavailable)?
    };
    tracing::debug!(
        user = user.sub,
        volume,
        path = path.as_str(),
        cached = hit,
        "thumbnail"
    );

    let content_type = HeaderValue::from_static(thumbnail.content_type);
    let mut res = Response::new(Body::from(thumbnail.bytes));
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    cache_headers(headers, etag);
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    Ok(res)
}

/// A source that is gone has no thumbnail; one that can't be read is refused like a download.
fn unreadable(volume: &str, path: &RelPath, err: StorageError) -> ApiError {
    match err {
        StorageError::NotFound(_) => ApiError::thumbnail_unavailable(),
        // A folder of unreadable images would log one warning per tile.
        err => {
            tracing::debug!(volume, path = path.as_str(), error = %err, "thumbnail source unreadable");
            ApiError::from_logged_storage(volume, err)
        }
    }
}

fn cache_headers(headers: &mut HeaderMap, etag: HeaderValue) {
    headers.insert(header::ETAG, etag);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
}

async fn lookup(thumbnails: &Arc<Thumbnails>, key: &str) -> Result<Option<Thumbnail>, ApiError> {
    let thumbnails = Arc::clone(thumbnails);
    let key = key.to_owned();
    let found = tokio::task::spawn_blocking(move || thumbnails.cached(&key))
        .await
        .map_err(ApiError::internal)?;
    Ok(found.unwrap_or_else(|err| {
        tracing::warn!(error = %err, "cannot read the thumbnail cache");
        None
    }))
}

/// Runs `work` on the blocking pool under a render permit. The permit moves into the task, so a
/// request cancelled mid-render (a grid tile scrolled away) still holds it until the work ends.
async fn bounded<T: Send + 'static>(
    thumbnails: &Thumbnails,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    let permit = thumbnails.permit().await;
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _entered = span.enter();
        work()
    })
    .await
    .map_err(ApiError::internal)
}

/// The file a thumbnail is asked for, as it was when its cache key was made.
struct Source {
    volumes: Arc<Volumes>,
    volume: String,
    path: RelPath,
    size: u64,
    modified: Option<SystemTime>,
}

impl Source {
    /// `None` if the file is gone or changed since the key was made.
    fn open(&self) -> Result<Option<File>, StorageError> {
        let Some(volume) = self.volumes.get(&self.volume) else {
            return Ok(None);
        };
        match volume.open_file(&self.path) {
            Ok((file, meta)) if meta.size == Some(self.size) && meta.modified == self.modified => {
                Ok(Some(file))
            }
            Ok(_) | Err(StorageError::NotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// Renders and caches the thumbnail, unless a concurrent request already did. `None` when the
/// file is gone, changed or can't be decoded; an error when it can't be opened.
fn generate(
    thumbnails: &Thumbnails,
    key: &str,
    source: &Source,
) -> Result<Option<Thumbnail>, StorageError> {
    if let Ok(Some(thumbnail)) = thumbnails.cached(key) {
        return Ok(Some(thumbnail));
    }
    let Some(file) = source.open()? else {
        return Ok(None);
    };
    let thumbnail = match render(file) {
        Ok(thumbnail) => thumbnail,
        Err(err) => {
            tracing::debug!(error = %err, "no thumbnail");
            return Ok(None);
        }
    };
    if let Err(err) = thumbnails.store(key, &thumbnail) {
        tracing::warn!(error = %err, "cannot cache a thumbnail");
    }
    Ok(Some(thumbnail))
}

/// Weak comparison (RFC 9110 section 13.1.2), as `If-None-Match` requires. `*` never matches:
/// the thumbnail may not exist yet, and a 304 would leave the client with nothing to show.
fn etag_matches(if_none_match: &str, key: &str) -> bool {
    if_none_match.split(',').map(str::trim).any(|tag| {
        tag.strip_prefix("W/")
            .unwrap_or(tag)
            .strip_prefix('"')
            .and_then(|tag| tag.strip_suffix('"'))
            == Some(key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbnails::THUMB_CONCURRENCY;

    #[test]
    fn if_none_match_lists_and_weak_tags() {
        assert!(etag_matches("\"abc\"", "abc"));
        assert!(etag_matches("W/\"abc\"", "abc"));
        assert!(etag_matches("\"x\", \"abc\"", "abc"));
        assert!(!etag_matches("*", "abc"));
        assert!(!etag_matches("\"abcd\"", "abc"));
        assert!(!etag_matches("abc", "abc"));
        assert!(!etag_matches("", "abc"));
    }

    #[tokio::test]
    async fn cancelled_request_keeps_its_permit_until_the_render_ends() {
        let dir = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(dir.path().to_owned()).unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let mut request = Box::pin(bounded(&thumbnails, move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        tokio::select! {
            _ = &mut request => panic!("the render was released early"),
            started = started_rx => started.unwrap(),
        }
        drop(request);
        assert_eq!(thumbnails.available_permits(), THUMB_CONCURRENCY - 1);

        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while thumbnails.available_permits() < THUMB_CONCURRENCY {
            assert!(
                std::time::Instant::now() < deadline,
                "the permit was never returned"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
}
