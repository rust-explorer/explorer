use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::http::error::ApiError;
use crate::http::request_id::request_id;
use crate::telemetry::truncate_for_log;

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
        offending = source
            .map(|v| truncate_for_log(&String::from_utf8_lossy(v.as_bytes())))
            .as_deref().unwrap_or("<none>"),
        "cross-origin request rejected"
    );
    ApiError::csrf_rejected().into_response()
}

/// Rejects requests a browser marks as sent from another site, such as a link or an image on
/// another site that would start a download or a zip. Sibling subdomains (`same-site`) are other
/// apps on a NAS, so they are refused too; a link the user clicks still downloads.
pub async fn refuse_cross_site(req: Request, next: Next) -> Response {
    if !is_foreign(req.method(), req.headers()) {
        return next.run(req).await;
    }
    tracing::warn!(
        request_id = request_id(req.headers()),
        method = %req.method(),
        path = req.uri().path(),
        "cross-site request rejected"
    );
    ApiError::csrf_rejected().into_response()
}

/// Whether the browser says another site sent the request, other than by a user's click on a
/// link. Without `Sec-Fetch-*` headers, nothing is known and the request passes.
fn is_foreign(method: &Method, headers: &HeaderMap) -> bool {
    let is = |name: &str, expected: &[u8]| {
        headers
            .get(name)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(expected))
    };
    let other_site = is("sec-fetch-site", b"cross-site") || is("sec-fetch-site", b"same-site");
    let clicked =
        is_safe(method) && is("sec-fetch-mode", b"navigate") && is("sec-fetch-user", b"?1");
    other_site && !clicked
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The origin of the public URL, as browsers send it in `Origin` and as the provider has it
/// registered for post-logout redirects (no trailing slash).
#[must_use]
pub fn public_origin(public_url: &url::Url) -> String {
    public_url.origin().ascii_serialization()
}

/// The serialised origin, or `None` for malformed or opaque (`null`) origins.
fn origin_of(value: &HeaderValue) -> Option<String> {
    let url = Url::parse(value.to_str().ok()?).ok()?;
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
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

    fn foreign(method: &Method, fetch: &[(&'static str, &'static str)]) -> bool {
        let mut headers = HeaderMap::new();
        for (name, value) in fetch {
            headers.insert(*name, HeaderValue::from_static(value));
        }
        is_foreign(method, &headers)
    }

    #[test]
    fn other_sites_are_foreign_unless_the_user_clicked_a_link() {
        let site = "sec-fetch-site";
        let clicked = [("sec-fetch-mode", "navigate"), ("sec-fetch-user", "?1")];
        for other in ["cross-site", "same-site", "Cross-Site"] {
            assert!(foreign(&Method::GET, &[(site, other)]), "{other}");
            assert!(
                foreign(
                    &Method::GET,
                    &[(site, other), ("sec-fetch-mode", "navigate")]
                ),
                "{other} without a click"
            );
            assert!(
                foreign(
                    &Method::GET,
                    &[(site, other), ("sec-fetch-mode", "no-cors"), clicked[1]]
                ),
                "{other} not a navigation"
            );
            assert!(
                foreign(&Method::POST, &[(site, other), clicked[0], clicked[1]]),
                "{other} form post"
            );
            assert!(
                !foreign(&Method::GET, &[(site, other), clicked[0], clicked[1]]),
                "{other} link"
            );
        }
        for own in ["same-origin", "none"] {
            assert!(!foreign(&Method::GET, &[(site, own)]), "{own}");
        }
        assert!(!foreign(&Method::GET, &[]));
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
}
