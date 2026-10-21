use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName};
use axum::middleware::Next;
use axum::response::Response;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

const MAX_REQUEST_ID_LEN: usize = 128;

#[must_use]
pub fn request_id(headers: &HeaderMap) -> &str {
    headers
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn is_valid_request_id(id: &[u8]) -> bool {
    (1..=MAX_REQUEST_ID_LEN).contains(&id.len())
        && id
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Removes a client-supplied request id that fails validation so `SetRequestIdLayer` generates one.
pub(super) async fn drop_invalid_request_id(mut request: Request, next: Next) -> Response {
    let invalid = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .is_some_and(|v| !is_valid_request_id(v.as_bytes()));
    if invalid {
        request.headers_mut().remove(REQUEST_ID_HEADER);
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_defaults_to_empty() {
        let mut headers = HeaderMap::new();
        assert_eq!(request_id(&headers), "");
        headers.insert("x-request-id", "abc".parse().unwrap());
        assert_eq!(request_id(&headers), "abc");
    }
}
