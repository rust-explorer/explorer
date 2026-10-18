use std::path::{Path, PathBuf};

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
                   img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; \
                   base-uri 'self'; form-action 'self'";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

#[derive(Debug, thiserror::Error)]
#[error("web bundle not found: {} must exist and be a file", .index.display())]
pub struct BundleError {
    index: PathBuf,
}

pub fn check_bundle(web_dir: &Path) -> Result<(), BundleError> {
    let index = web_dir.join("index.html");
    if index.is_file() {
        Ok(())
    } else {
        Err(BundleError { index })
    }
}

/// Serves the bundle, falling back to `index.html` for client-side routes.
/// Missing files under `/assets/` are real 404s so a stale hash never yields HTML.
pub fn service(web_dir: &Path) -> Router {
    let index = ServeFile::new(web_dir.join("index.html"));
    let files = ServeDir::new(web_dir).fallback(index);
    Router::new()
        .nest_service("/assets", ServeDir::new(web_dir.join("assets")))
        .fallback_service(files)
        .layer(middleware::from_fn(cache_control))
}

async fn cache_control(req: Request, next: Next) -> Response {
    let hashed = req.uri().path().starts_with("/assets/");
    let mut res = next.run(req).await;
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    if res.status().is_success() {
        if hashed {
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
        } else if is_html {
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        }
    }
    res
}

pub fn security_headers(router: Router) -> Router {
    router
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
}
