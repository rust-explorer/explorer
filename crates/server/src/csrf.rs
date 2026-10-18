use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::error::ApiError;

const MAX_LOGGED_LEN: usize = 256;

/// Rejects state-changing requests whose `Origin` (or, failing that, `Referer`) is not `origin`.
pub async fn require_same_origin(
    State(origin): State<Arc<str>>,
    req: Request,
    next: Next,
) -> Response {
    if is_safe(req.method()) {
        return next.run(req).await;
    }
    let source = req
        .headers()
        .get(header::ORIGIN)
        .or_else(|| req.headers().get(header::REFERER));
    if source.is_some_and(|value| origin_of(value).as_deref() == Some(&*origin)) {
        return next.run(req).await;
    }
    tracing::warn!(
        request_id = request_id(req.headers()),
        method = %req.method(),
        offending = source.map(truncated).as_deref().unwrap_or("<none>"),
        "cross-origin request rejected"
    );
    ApiError::csrf_rejected().into_response()
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The serialised origin, or `None` for malformed or opaque (`null`) origins.
fn origin_of(value: &HeaderValue) -> Option<String> {
    let url = Url::parse(value.to_str().ok()?).ok()?;
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

fn request_id(headers: &HeaderMap) -> &str {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn truncated(value: &HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes())
        .chars()
        .take(MAX_LOGGED_LEN)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(value: &str) -> Option<String> {
        origin_of(&HeaderValue::from_str(value).unwrap())
    }

    #[test]
    fn origin_drops_path_and_default_port() {
        assert_eq!(
            origin("https://files.example.com:443/a?b").as_deref(),
            Some("https://files.example.com")
        );
    }

    #[test]
    fn origin_keeps_explicit_port() {
        assert_eq!(
            origin("http://localhost:8080").as_deref(),
            Some("http://localhost:8080")
        );
    }

    #[test]
    fn opaque_and_malformed_origins_are_none() {
        assert_eq!(origin("null"), None);
        assert_eq!(origin("data:text/plain,x"), None);
        assert_eq!(origin("files.example.com"), None);
        assert_eq!(origin(""), None);
    }

    #[test]
    fn non_utf8_origin_is_none() {
        assert_eq!(
            origin_of(&HeaderValue::from_bytes(b"https://\xffx").unwrap()),
            None
        );
    }

    #[test]
    fn only_get_head_options_are_safe() {
        assert!(is_safe(&Method::GET));
        assert!(is_safe(&Method::HEAD));
        assert!(is_safe(&Method::OPTIONS));
        for m in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::TRACE,
        ] {
            assert!(!is_safe(&m));
        }
    }

    #[test]
    fn logged_value_is_truncated() {
        let long = HeaderValue::from_str(&"a".repeat(1000)).unwrap();
        assert_eq!(truncated(&long).len(), MAX_LOGGED_LEN);
    }
}
