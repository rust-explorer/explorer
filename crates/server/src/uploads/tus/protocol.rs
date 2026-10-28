use axum::extract::Request;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::header::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use shared::api::conflict::ConflictPolicy;
use shared::tus::{TUS_VERSION, decode_metadata};

use crate::http::error::ApiError;

pub(super) const OFFSET_STREAM: &str = "application/offset+octet-stream";
pub(super) const EXTENSIONS: &str = "creation,termination";

pub(super) const TUS_RESUMABLE: HeaderName = HeaderName::from_static("tus-resumable");
pub(super) const TUS_VERSION_HEADER: HeaderName = HeaderName::from_static("tus-version");
pub(super) const TUS_EXTENSION: HeaderName = HeaderName::from_static("tus-extension");
pub(super) const UPLOAD_LENGTH: HeaderName = HeaderName::from_static("upload-length");
pub(super) const UPLOAD_OFFSET: HeaderName = HeaderName::from_static("upload-offset");
pub(super) const UPLOAD_METADATA: HeaderName = HeaderName::from_static("upload-metadata");

/// Sends `Tus-Resumable` on every response under `/files/uploads`. It sits in front of the
/// CSRF check, and also covers paths the uploads router does not match.
pub async fn tag_tus_responses(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let tus = path == "/files/uploads" || path.starts_with("/files/uploads/");
    let mut res = next.run(req).await;
    if tus {
        res.headers_mut()
            .insert(TUS_RESUMABLE, HeaderValue::from_static(TUS_VERSION));
    }
    res
}

/// Requires `Tus-Resumable` on every request but `OPTIONS`.
pub(super) async fn require_tus_version(req: Request, next: Next) -> Response {
    let supported = req.method() == Method::OPTIONS
        || req
            .headers()
            .get(&TUS_RESUMABLE)
            .is_some_and(|v| v.as_bytes() == TUS_VERSION.as_bytes());
    if supported {
        return next.run(req).await;
    }
    let mut res = ApiError::unsupported_tus_version().into_response();
    res.headers_mut()
        .insert(TUS_VERSION_HEADER, HeaderValue::from_static(TUS_VERSION));
    res
}

pub(super) struct Creation {
    pub(super) volume: String,
    pub(super) path: String,
    pub(super) length: u64,
    pub(super) modified_ms: Option<i64>,
    pub(super) conflict: ConflictPolicy,
}

pub(super) fn parse_creation(headers: &HeaderMap) -> Result<Creation, ApiError> {
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

pub(super) fn parse_decimal(value: &HeaderValue) -> Option<u64> {
    let value = value.to_str().ok()?;
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(super) fn no_content(offset: u64) -> Response {
    (
        StatusCode::NO_CONTENT,
        [(UPLOAD_OFFSET, offset.to_string())],
    )
        .into_response()
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
}
