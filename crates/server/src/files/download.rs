use std::io::SeekFrom;
use std::ops::RangeInclusive;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::api::blocking_volume;
use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::authz::{Action, Policy};
use crate::error::ApiError;
use crate::storage::RelPath;

/// RFC 8187 `attr-char`: everything else is percent-encoded.
const FILENAME_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

#[derive(Deserialize)]
pub struct DownloadQuery {
    volume: String,
    path: String,
}

#[must_use]
pub fn content_disposition(file_name: &str) -> HeaderValue {
    let fallback: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded = utf8_percent_encode(file_name, FILENAME_ENCODE);
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// Headers that keep a user file from ever being rendered in the app's origin.
pub fn attachment_headers(headers: &mut HeaderMap, file_name: &str) {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(header::CONTENT_DISPOSITION, content_disposition(file_name));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
}

pub async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<DownloadQuery>,
    request_headers: HeaderMap,
) -> Result<Response, ApiError> {
    let DownloadQuery { volume, path } = query;
    if state.volumes.get(&volume).is_none() {
        return Err(ApiError::volume_not_found(&volume));
    }
    Policy.check(&user, &volume, Action::Read)?;
    let path = RelPath::parse(&path)?;

    let (file, meta) = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.open_file(&path)
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;

    let size = meta.size.unwrap_or_default();
    let validators = Validators::new(size, meta.modified);
    let range_header = request_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    tracing::info!(
        user = user.sub,
        action = "download",
        volume,
        path = path.as_str(),
        range = range_header.unwrap_or("-"),
        "download"
    );

    let Ok(selected) = select_range(&request_headers, size, &validators) else {
        return Ok(range_not_satisfiable(&volume, &path, size));
    };

    let mut file = tokio::fs::File::from_std(file);
    let (status, start, length) = match &selected {
        Some(range) => {
            file.seek(SeekFrom::Start(*range.start()))
                .await
                .map_err(ApiError::internal)?;
            (
                StatusCode::PARTIAL_CONTENT,
                *range.start(),
                range.end() - range.start() + 1,
            )
        }
        None => (StatusCode::OK, 0, size),
    };

    let mut res = Response::new(Body::from_stream(ReaderStream::new(file.take(length))));
    *res.status_mut() = status;
    let headers = res.headers_mut();
    attachment_headers(headers, meta.name.as_str());
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if selected.is_some()
        && let Ok(value) =
            HeaderValue::from_str(&format!("bytes {start}-{}/{size}", start + length - 1))
    {
        headers.insert(header::CONTENT_RANGE, value);
    }
    validators.apply(headers);
    Ok(res)
}

fn range_not_satisfiable(volume: &str, path: &RelPath, size: u64) -> Response {
    let message = format!("Range not satisfiable on {volume}/{}", path.as_str());
    let mut res = ApiError::new(
        StatusCode::RANGE_NOT_SATISFIABLE,
        "range_not_satisfiable",
        message,
    )
    .with_details(serde_json::json!({ "volume": volume, "path": path.as_str() }))
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
        res.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    res
}

/// The weak `ETag` and `Last-Modified` of a file; both are absent when it has no mtime.
struct Validators {
    etag: Option<String>,
    modified: Option<SystemTime>,
}

impl Validators {
    fn new(size: u64, modified: Option<SystemTime>) -> Self {
        let nanos = modified
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos());
        let Some(nanos) = nanos else {
            return Self {
                etag: None,
                modified: None,
            };
        };
        Self {
            etag: Some(format!("W/\"{size}-{nanos:x}\"")),
            // HTTP dates have one-second resolution
            modified: modified.map(|time| {
                let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                UNIX_EPOCH + Duration::from_secs(secs)
            }),
        }
    }

    fn apply(&self, headers: &mut HeaderMap) {
        if let Some(value) = self
            .etag
            .as_deref()
            .and_then(|etag| HeaderValue::from_str(etag).ok())
        {
            headers.insert(header::ETAG, value);
        }
        if let Some(value) = self
            .modified
            .and_then(|time| HeaderValue::from_str(&httpdate::fmt_http_date(time)).ok())
        {
            headers.insert(header::LAST_MODIFIED, value);
        }
    }

    fn matches_if_range(&self, value: &str) -> bool {
        let value = value.trim();
        if value.starts_with('"') || value.starts_with("W/") {
            return self.etag.as_deref() == Some(value);
        }
        match (httpdate::parse_http_date(value), self.modified) {
            (Ok(date), Some(modified)) => date == modified,
            _ => false,
        }
    }
}

struct Unsatisfiable;

/// `Ok(None)` serves the whole file. A malformed `Range`, a multi-range and a stale `If-Range`
/// all fall back to the full file, as the RFC allows.
fn select_range(
    headers: &HeaderMap,
    size: u64,
    validators: &Validators,
) -> Result<Option<RangeInclusive<u64>>, Unsatisfiable> {
    let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    if let Some(if_range) = headers.get(header::IF_RANGE) {
        let fresh = if_range
            .to_str()
            .is_ok_and(|value| validators.matches_if_range(value));
        if !fresh {
            return Ok(None);
        }
    }
    let Ok(parsed) = http_range_header::parse_range_header(range) else {
        return Ok(None);
    };
    let mut ranges = parsed.validate(size).map_err(|_| Unsatisfiable)?;
    if ranges.len() == 1 {
        Ok(ranges.pop())
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_encodes_unicode() {
        assert_eq!(
            content_disposition("Été 😀.mkv"),
            "attachment; filename=\"_t_ _.mkv\"; filename*=UTF-8''%C3%89t%C3%A9%20%F0%9F%98%80.mkv"
        );
    }

    #[test]
    fn content_disposition_neutralises_quotes_and_backslashes() {
        let value = content_disposition("a\"b\\c.txt");
        assert_eq!(
            value,
            "attachment; filename=\"a_b_c.txt\"; filename*=UTF-8''a%22b%5Cc.txt"
        );
    }
}
