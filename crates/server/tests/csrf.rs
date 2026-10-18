mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use explorer_server::app::{AppState, router_with};
use http_body_util::BodyExt;
use shared::api::ErrorBody;
use tower::ServiceExt;

const PUBLIC_ORIGIN: &str = "https://files.example.com";

async fn echo() -> &'static str {
    "ok"
}

async fn app() -> (Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let extra: Router<AppState> = Router::new().route("/api/_echo", post(echo).get(echo));
    let app = router_with(common::state_with_web_dir(dir.path()).await, extra)
        .await
        .unwrap();
    (app, dir)
}

fn request(method: &str, headers: &[(header::HeaderName, &str)]) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri("/api/_echo");
    for (name, value) in headers {
        builder = builder.header(name, *value);
    }
    builder.body(Body::empty()).unwrap()
}

async fn send(req: Request<Body>) -> axum::response::Response {
    let (app, _dir) = app().await;
    app.oneshot(req).await.unwrap()
}

async fn assert_rejected(req: Request<Body>) {
    let res = send(req).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.code, "csrf_rejected");
    assert_eq!(body.request_id, Some(id));
}

#[tokio::test]
async fn post_with_matching_origin_passes() {
    let res = send(request("POST", &[(header::ORIGIN, PUBLIC_ORIGIN)])).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_with_foreign_origin_is_rejected() {
    assert_rejected(request("POST", &[(header::ORIGIN, "https://evil.example")])).await;
}

#[tokio::test]
async fn post_with_matching_referer_and_no_origin_passes() {
    let referer = format!("{PUBLIC_ORIGIN}/some/page?q=1");
    let res = send(request("POST", &[(header::REFERER, &referer)])).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_without_origin_or_referer_is_rejected() {
    assert_rejected(request("POST", &[])).await;
}

#[tokio::test]
async fn get_is_never_checked() {
    let res = send(request("GET", &[(header::ORIGIN, "https://evil.example")])).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn origin_null_is_rejected() {
    assert_rejected(request("POST", &[(header::ORIGIN, "null")])).await;
}

#[tokio::test]
async fn foreign_origin_wins_over_matching_referer() {
    let referer = format!("{PUBLIC_ORIGIN}/");
    assert_rejected(request(
        "POST",
        &[
            (header::ORIGIN, "https://evil.example"),
            (header::REFERER, &referer),
        ],
    ))
    .await;
}

#[tokio::test]
async fn origin_with_other_port_or_scheme_is_rejected() {
    assert_rejected(request(
        "POST",
        &[(header::ORIGIN, "https://files.example.com:8443")],
    ))
    .await;
    assert_rejected(request(
        "POST",
        &[(header::ORIGIN, "http://files.example.com")],
    ))
    .await;
}

#[tokio::test]
async fn foreign_referer_is_rejected() {
    assert_rejected(request(
        "POST",
        &[(header::REFERER, "https://evil.example/files.example.com")],
    ))
    .await;
}

#[tokio::test]
async fn malformed_origin_is_rejected() {
    assert_rejected(request("POST", &[(header::ORIGIN, "not a url")])).await;
}

#[tokio::test]
async fn cross_site_form_post_to_logout_is_rejected() {
    let req = Request::post("/auth/logout")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, "explorer_session=whatever")
        .body(Body::empty())
        .unwrap();
    let (app, _dir) = app().await;
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.code, "csrf_rejected");
}
