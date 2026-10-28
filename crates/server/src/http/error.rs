use axum::Json;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use shared::api::error::ErrorBody;
use shared::api::favorites::MAX_FAVORITES;
use shared::api::tasks::MAX_ACTIVE_TASKS;

use crate::http::request_id::request_id;
use crate::storage::{InvalidPath, StorageError};

const UNUSABLE_STAGED: &str = "The staged upload can't be used";

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<serde_json::Value>,
}

impl ApiError {
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "Resource not found")
    }

    #[must_use]
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Authentication required",
        )
    }

    #[must_use]
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "Access denied")
    }

    #[must_use]
    pub fn csrf_rejected() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "csrf_rejected",
            "Cross-origin request rejected",
        )
    }

    #[must_use]
    pub fn volume_not_found(name: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "volume_not_found",
            format!("Volume not found: {name}"),
        )
        .with_details(json!({ "volume": name }))
    }

    #[must_use]
    pub fn read_only_volume(name: &str) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "read_only_volume",
            format!("Volume {name} is read-only"),
        )
        .with_details(json!({ "volume": name }))
    }

    #[must_use]
    pub fn insufficient_storage(volume: &str, path: &str) -> Self {
        Self::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "insufficient_storage",
            format!("Not enough space on {volume}"),
        )
        .with_details(json!({ "volume": volume, "path": path }))
    }

    #[must_use]
    pub fn upload_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "upload_not_found",
            "Upload not found",
        )
    }

    #[must_use]
    pub fn favorites_full() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "favorites_full",
            format!("Favorites are limited to {MAX_FAVORITES}"),
        )
    }

    #[must_use]
    pub fn thumbnail_unavailable() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "thumbnail_unavailable",
            "No thumbnail for this file",
        )
    }

    #[must_use]
    pub fn task_not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "task_not_found", "Task not found")
    }

    #[must_use]
    pub fn too_many_tasks() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_tasks",
            format!("At most {MAX_ACTIVE_TASKS} tasks can wait or run at once"),
        )
    }

    /// `offset` is where the upload actually stands.
    #[must_use]
    pub fn offset_mismatch(offset: u64) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "offset_mismatch",
            format!("The upload is at offset {offset}"),
        )
        .with_details(json!({ "offset": offset }))
    }

    #[must_use]
    pub fn upload_busy() -> Self {
        Self::new(
            StatusCode::LOCKED,
            "upload_busy",
            "Another request is writing this upload",
        )
    }

    #[must_use]
    pub fn upload_too_large(length: u64) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            format!("The upload is limited to {length} bytes"),
        )
        .with_details(json!({ "length": length }))
    }

    #[must_use]
    pub fn invalid_upload(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_upload", message)
    }

    #[must_use]
    pub fn unsupported_tus_version() -> Self {
        Self::new(
            StatusCode::PRECONDITION_FAILED,
            "unsupported_tus_version",
            "Unsupported tus version",
        )
    }

    #[must_use]
    pub fn unsupported_media_type(expected: &str) -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            format!("Expected Content-Type {expected}"),
        )
    }

    /// Maps a storage failure on `volume`, logging it; an I/O error stays out of the response.
    #[must_use]
    pub fn from_storage(volume: &str, err: StorageError) -> Self {
        Self::map_storage(volume, err, true)
    }

    /// Like [`ApiError::from_storage`], for a failure the caller logged with its user and action.
    #[must_use]
    pub fn from_logged_storage(volume: &str, err: StorageError) -> Self {
        Self::map_storage(volume, err, false)
    }

    fn map_storage(volume: &str, err: StorageError, log: bool) -> Self {
        let (status, code, label, path) = match err {
            StorageError::NotFound(path) => {
                (StatusCode::NOT_FOUND, "path_not_found", "Not found: ", path)
            }
            StorageError::NotADirectory(path) => (
                StatusCode::BAD_REQUEST,
                "not_a_directory",
                "Not a folder: ",
                path,
            ),
            StorageError::IsADirectory(path) => (
                StatusCode::BAD_REQUEST,
                "is_a_directory",
                "Is a folder: ",
                path,
            ),
            StorageError::PermissionDenied(path) => (
                StatusCode::FORBIDDEN,
                "permission_denied",
                "Permission denied on ",
                path,
            ),
            StorageError::AlreadyExists(path) => (
                StatusCode::CONFLICT,
                "already_exists",
                "Already exists: ",
                path,
            ),
            StorageError::NoSpace(path) => {
                if log {
                    tracing::warn!(volume, path = path.as_str(), "not enough space");
                }
                return Self::insufficient_storage(volume, path.as_str());
            }
            StorageError::OutsideVolume(path) => {
                if log {
                    tracing::warn!(
                        volume,
                        path = path.as_str(),
                        "path points outside the volume"
                    );
                }
                let message = format!(
                    "{} points outside the volume",
                    display_path(volume, path.as_str())
                );
                return Self::new(StatusCode::FORBIDDEN, "outside_volume", message)
                    .with_details(json!({ "volume": volume, "path": path.as_str() }));
            }
            StorageError::CrossDevice(path) => {
                if log {
                    tracing::warn!(
                        volume,
                        path = path.as_str(),
                        "the destination is on another filesystem than the volume; mount it as its own volume"
                    );
                }
                let message = format!(
                    "{} is on another filesystem than the volume",
                    display_path(volume, path.as_str())
                );
                return Self::new(StatusCode::INTERNAL_SERVER_ERROR, "cross_device", message)
                    .with_details(json!({ "volume": volume, "path": path.as_str() }));
            }
            StorageError::InvalidDestination(path) => {
                if log {
                    tracing::warn!(volume, path = path.as_str(), "invalid destination");
                }
                let message = format!(
                    "{} can't be moved or copied there",
                    display_path(volume, path.as_str())
                );
                return Self::new(StatusCode::BAD_REQUEST, "invalid_destination", message)
                    .with_details(json!({ "volume": volume, "path": path.as_str() }));
            }
            // The caller picks the source volume for it, see [`ApiError::from_transfer`].
            StorageError::Source(inner) => return Self::map_storage(volume, *inner, log),
            StorageError::InvalidUpload => return Self::invalid_upload(UNUSABLE_STAGED),
            StorageError::Cancelled => {
                return Self::internal("a cancellation reported as an error");
            }
            StorageError::InvalidPath(err) => {
                if log {
                    tracing::warn!(volume, reason = err.reason, "invalid path");
                }
                return err.into();
            }
            StorageError::Io { path, source } => {
                if !log {
                    return Self::internal_error();
                }
                tracing::warn!(volume, path = path.as_str(), error = %source, "storage error");
                return Self::internal(source);
            }
        };
        if log {
            tracing::warn!(volume, path = path.as_str(), code, "storage request failed");
        }
        let message = format!("{label}{}", display_path(volume, path.as_str()));
        Self::new(status, code, message)
            .with_details(json!({ "volume": volume, "path": path.as_str() }))
    }

    /// Maps a failure of a copy or move with the volume it happened on; an invalid destination
    /// names the source. The caller logs the failure with its user and action.
    #[must_use]
    pub fn from_transfer(source: &str, dest: &str, err: StorageError) -> Self {
        match err {
            StorageError::Source(inner) => Self::from_logged_storage(source, *inner),
            err @ StorageError::InvalidDestination(_) => Self::from_logged_storage(source, err),
            other => Self::from_logged_storage(dest, other),
        }
    }

    /// The cause is logged, never returned to the client.
    #[must_use]
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::internal_error()
    }

    fn internal_error() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Internal server error",
        )
    }
}

impl ApiError {
    /// The JSON body this error answers with, for a failure reported later, as a task's.
    #[must_use]
    pub fn to_body(&self, request_id: Option<String>) -> ErrorBody {
        ErrorBody {
            code: self.code.to_owned(),
            message: self.message.clone(),
            details: self.details.clone(),
            request_id,
        }
    }
}

impl From<InvalidPath> for ApiError {
    fn from(err: InvalidPath) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            format!("Invalid path: {}", err.reason),
        )
    }
}

fn display_path(volume: &str, path: &str) -> String {
    if path.is_empty() {
        volume.to_owned()
    } else {
        format!("{volume}/{path}")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = self.to_body(None);
        let mut response = (self.status, Json(body.clone())).into_response();
        // Lets `attach_request_id` rebuild the body once the request id is known.
        response.extensions_mut().insert(body);
        response
    }
}

pub async fn attach_request_id(request: Request, next: Next) -> Response {
    let request_id = Some(request_id(request.headers()))
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let mut response = next.run(request).await;
    let Some(mut body) = response.extensions_mut().remove::<ErrorBody>() else {
        return response;
    };
    body.request_id = request_id;
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (parts, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RelPath;

    #[test]
    fn cross_device_is_not_internal() {
        let err = ApiError::from_storage(
            "media",
            StorageError::CrossDevice(RelPath::parse("Mounted/a.txt").unwrap()),
        );
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.code, "cross_device");
        assert_eq!(
            err.details,
            Some(json!({ "volume": "media", "path": "Mounted/a.txt" }))
        );
    }

    #[test]
    fn odd_staged_upload_is_a_client_error() {
        let err = ApiError::from_storage("media", StorageError::InvalidUpload);
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_upload");
    }

    #[test]
    fn body_carries_the_request_id() {
        let body = ApiError::from_storage(
            "media",
            StorageError::NotFound(RelPath::parse("a.txt").unwrap()),
        )
        .to_body(Some("req-1".to_owned()));
        assert_eq!(body.code, "path_not_found");
        assert_eq!(body.message, "Not found: media/a.txt");
        assert_eq!(
            body.details,
            Some(json!({ "volume": "media", "path": "a.txt" }))
        );
        assert_eq!(body.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn transfer_errors_name_the_volume_they_happened_on() {
        let path = || RelPath::parse("a.txt").unwrap();
        let source = ApiError::from_transfer(
            "media",
            "documents",
            StorageError::Source(Box::new(StorageError::PermissionDenied(path()))),
        );
        assert_eq!(source.code, "permission_denied");
        assert_eq!(
            source.details,
            Some(json!({ "volume": "media", "path": "a.txt" }))
        );
        let dest =
            ApiError::from_transfer("media", "documents", StorageError::AlreadyExists(path()));
        assert_eq!(dest.code, "already_exists");
        assert_eq!(
            dest.details,
            Some(json!({ "volume": "documents", "path": "a.txt" }))
        );
    }

    #[test]
    fn invalid_destination_names_the_source() {
        let err = ApiError::from_transfer(
            "media",
            "documents",
            StorageError::InvalidDestination(RelPath::parse("Films").unwrap()),
        );
        assert_eq!(
            err.details,
            Some(json!({ "volume": "media", "path": "Films" }))
        );
    }

    #[test]
    fn invalid_destination_is_bad_request() {
        let err = ApiError::from_storage(
            "media",
            StorageError::InvalidDestination(RelPath::parse("Films").unwrap()),
        );
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "invalid_destination");
        assert_eq!(
            err.details,
            Some(json!({ "volume": "media", "path": "Films" }))
        );
    }
}
