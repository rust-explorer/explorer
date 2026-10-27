use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::Response;

use super::{MAX_SOURCE_BYTES, Thumbnail, Thumbnails, cache_key, render, supported};
use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::{VolumeQuery, authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::StorageError;

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
    if !path.file_name().is_some_and(supported) {
        return Err(ApiError::thumbnail_unavailable());
    }

    let (file, meta) = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.open_file(&path)
    })
    .await?
    .map_err(|err| match err {
        StorageError::NotFound(_) | StorageError::IsADirectory(_) => {
            ApiError::thumbnail_unavailable()
        }
        err => ApiError::from_storage(&volume, err),
    })?;
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
        let _permit = state.thumbnails.permit().await;
        let thumbnails = Arc::clone(&state.thumbnails);
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            generate(&thumbnails, &key, file)
        })
        .await
        .map_err(ApiError::internal)?
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

/// Renders and caches the thumbnail, unless a concurrent request already did. `None` when the
/// file can't be decoded.
fn generate(thumbnails: &Thumbnails, key: &str, file: std::fs::File) -> Option<Thumbnail> {
    if let Ok(Some(thumbnail)) = thumbnails.cached(key) {
        return Some(thumbnail);
    }
    let thumbnail = match render(file) {
        Ok(thumbnail) => thumbnail,
        Err(err) => {
            tracing::debug!(error = %err, "no thumbnail");
            return None;
        }
    };
    if let Err(err) = thumbnails.store(key, &thumbnail) {
        tracing::warn!(error = %err, "cannot cache a thumbnail");
    }
    Some(thumbnail)
}

/// Weak comparison (RFC 9110 section 13.1.2), as `If-None-Match` requires.
fn etag_matches(if_none_match: &str, key: &str) -> bool {
    if_none_match.split(',').map(str::trim).any(|tag| {
        tag == "*"
            || tag
                .strip_prefix("W/")
                .unwrap_or(tag)
                .strip_prefix('"')
                .and_then(|tag| tag.strip_suffix('"'))
                == Some(key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_none_match_lists_and_weak_tags() {
        assert!(etag_matches("\"abc\"", "abc"));
        assert!(etag_matches("W/\"abc\"", "abc"));
        assert!(etag_matches("\"x\", \"abc\"", "abc"));
        assert!(etag_matches("*", "abc"));
        assert!(!etag_matches("\"abcd\"", "abc"));
        assert!(!etag_matches("abc", "abc"));
        assert!(!etag_matches("", "abc"));
    }
}
