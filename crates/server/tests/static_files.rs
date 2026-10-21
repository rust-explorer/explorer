mod common;

use axum::http::{StatusCode, header};
use common::{app_with_web_dir, get};
use explorer_server::http::static_files::check_bundle;
use http_body_util::BodyExt;
use shared::api::ErrorBody;
use tower::ServiceExt;

fn bundle() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>app</html>").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app-abc.js"), "console.log(1)").unwrap();
    dir
}

async fn body_string(res: axum::response::Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn serves_index_at_root() {
    let dir = bundle();
    let res = app_with_web_dir(dir.path())
        .await
        .oneshot(get("/"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(body_string(res).await.contains("app"));
}

#[tokio::test]
async fn serves_asset_with_immutable_cache() {
    let dir = bundle();
    let res = app_with_web_dir(dir.path())
        .await
        .oneshot(get("/assets/app-abc.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn missing_asset_is_404_not_index() {
    let dir = bundle();
    let res = app_with_web_dir(dir.path())
        .await
        .oneshot(get("/assets/missing.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_client_route_falls_back_to_index() {
    let dir = bundle();
    let res = app_with_web_dir(dir.path())
        .await
        .oneshot(get("/media/Films"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(body_string(res).await, "<html>app</html>");
}

#[tokio::test]
async fn api_and_auth_routes_do_not_fall_back() {
    let dir = bundle();
    for uri in ["/api/x", "/auth/x", "/api", "/auth", "/api/", "/auth/"] {
        let res = app_with_web_dir(dir.path())
            .await
            .oneshot(get(uri))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.code, "not_found", "{uri}");
        assert_eq!(body.request_id, Some(id), "{uri}");
    }
}

#[tokio::test]
async fn security_headers_present() {
    let dir = bundle();
    for uri in ["/", "/assets/app-abc.js", "/api/x", "/healthz"] {
        let res = app_with_web_dir(dir.path())
            .await
            .oneshot(get(uri))
            .await
            .unwrap();
        let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'"), "{uri}");
        assert!(
            csp.contains("script-src 'self' 'wasm-unsafe-eval'"),
            "{uri}"
        );
        assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(res.headers()[header::REFERRER_POLICY], "same-origin");
    }
}

#[test]
fn check_bundle_accepts_valid_bundle() {
    let dir = bundle();
    check_bundle(dir.path()).unwrap();
}

#[test]
fn check_bundle_fails_on_missing_index() {
    let dir = tempfile::tempdir().unwrap();
    let err = check_bundle(dir.path()).unwrap_err();
    assert!(err.to_string().contains(&dir.path().display().to_string()));
}

#[test]
fn check_bundle_fails_when_index_is_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("index.html")).unwrap();
    assert!(check_bundle(dir.path()).is_err());
}

#[tokio::test]
async fn missing_asset_is_bare_404() {
    let dir = bundle();
    let res = app_with_web_dir(dir.path())
        .await
        .oneshot(get("/assets/nope-123.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert!(!body_string(res).await.contains("<html"));
}

#[tokio::test]
async fn files_prefix_is_reserved() {
    let dir = bundle();
    for uri in ["/files/unknown", "/files/"] {
        let res = app_with_web_dir(dir.path())
            .await
            .oneshot(get(uri))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.code, "not_found", "{uri}");
    }
}
