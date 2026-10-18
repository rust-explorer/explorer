mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::get;
use http_body_util::BodyExt;
use shared::api::{ErrorBody, Health};
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (common::app_with_web_dir(dir.path()), dir)
}

#[tokio::test]
async fn healthz_returns_ok_json() {
    let (app, _dir) = app();
    let res = app.oneshot(get("/healthz")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let health: Health = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        health,
        Health {
            status: "ok".into(),
            version: env!("CARGO_PKG_VERSION").into()
        }
    );
}

#[tokio::test]
async fn responses_carry_request_id() {
    let (app, _dir) = app();
    let res = app.oneshot(get("/healthz")).await.unwrap();
    let id = res.headers().get("x-request-id").unwrap();
    assert!(!id.is_empty());
}

#[tokio::test]
async fn incoming_request_id_is_preserved() {
    let (app, _dir) = app();
    let req = Request::get("/healthz")
        .header("x-request-id", "abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.headers()["x-request-id"], "abc");
}

#[tokio::test]
async fn unknown_api_route_returns_json_404() {
    let (app, _dir) = app();
    let res = app.oneshot(get("/api/nope")).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.code, "not_found");
    assert_eq!(body.request_id, Some(id));
}
