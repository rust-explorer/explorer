use axum::Json;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use shared::api::ErrorBody;

use crate::storage::{InvalidPath, StorageError};

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
                tracing::warn!(volume, path = path.as_str(), "not enough space");
                return Self::insufficient_storage(volume, path.as_str());
            }
            StorageError::OutsideVolume(path) => {
                tracing::warn!(
                    volume,
                    path = path.as_str(),
                    "path points outside the volume"
                );
                let message = format!(
                    "{} points outside the volume",
                    display_path(volume, path.as_str())
                );
                return Self::new(StatusCode::FORBIDDEN, "outside_volume", message)
                    .with_details(json!({ "volume": volume, "path": path.as_str() }));
            }
            StorageError::CrossDevice(path) => {
                tracing::warn!(
                    volume,
                    path = path.as_str(),
                    "the destination is on another filesystem than the volume; mount it as its own volume"
                );
                let message = format!(
                    "{} is on another filesystem than the volume",
                    display_path(volume, path.as_str())
                );
                return Self::new(StatusCode::INTERNAL_SERVER_ERROR, "cross_device", message)
                    .with_details(json!({ "volume": volume, "path": path.as_str() }));
            }
            StorageError::Io { path, source } => {
                tracing::warn!(volume, path = path.as_str(), error = %source, "storage error");
                return Self::internal(source);
            }
        };
        tracing::warn!(volume, path = path.as_str(), code, "storage request failed");
        let message = format!("{label}{}", display_path(volume, path.as_str()));
        Self::new(status, code, message)
            .with_details(json!({ "volume": volume, "path": path.as_str() }))
    }

    /// The cause is logged, never returned to the client.
    #[must_use]
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Internal server error",
        )
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
        let body = ErrorBody {
            code: self.code.to_owned(),
            message: self.message,
            details: self.details,
            request_id: None,
        };
        let mut response = (self.status, Json(body.clone())).into_response();
        // Lets `attach_request_id` rebuild the body once the request id is known.
        response.extensions_mut().insert(body);
        response
    }
}

pub async fn attach_request_id(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
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
}
