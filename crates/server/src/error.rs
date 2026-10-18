use axum::Json;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use shared::api::ErrorBody;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    #[must_use]
    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "Resource not found".to_owned(),
        }
    }

    #[must_use]
    pub fn unauthenticated() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthenticated",
            message: "Authentication required".to_owned(),
        }
    }

    #[must_use]
    pub fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: "Access denied".to_owned(),
        }
    }

    #[must_use]
    pub fn csrf_rejected() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "csrf_rejected",
            message: "Cross-origin request rejected".to_owned(),
        }
    }

    /// The cause is logged, never returned to the client.
    #[must_use]
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: "Internal server error".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.code.to_owned(),
            message: self.message,
            details: None,
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
