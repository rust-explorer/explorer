mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::get;
use explorer_server::app::{AppState, router_with};
use explorer_server::error::ApiError;
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

async fn id_for_incoming(incoming: &str) -> String {
    let (app, _dir) = app();
    let req = Request::get("/healthz")
        .header("x-request-id", incoming)
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    res.headers()["x-request-id"].to_str().unwrap().to_owned()
}

#[tokio::test]
async fn overlong_request_id_is_replaced() {
    let incoming = "a".repeat(129);
    let id = id_for_incoming(&incoming).await;
    assert_ne!(id, incoming);
    assert_eq!(id.len(), 36);
}

#[tokio::test]
async fn request_id_with_invalid_chars_is_replaced() {
    for incoming in ["has space", "<script>", ""] {
        let id = id_for_incoming(incoming).await;
        assert_ne!(id, incoming);
        assert_eq!(id.len(), 36);
    }
}

#[tokio::test]
async fn request_id_at_length_limit_is_preserved() {
    let incoming = "a".repeat(128);
    assert_eq!(id_for_incoming(&incoming).await, incoming);
}

async fn boom() -> ApiError {
    ApiError::internal("db exploded")
}

#[tokio::test]
async fn internal_error_is_generic_and_carries_request_id() {
    let dir = tempfile::tempdir().unwrap();
    let extra: Router<AppState> = Router::new().route("/api/_boom", axum::routing::get(boom));
    let app = router_with(common::state_with_web_dir(dir.path()), extra);
    let res = app.oneshot(get("/api/_boom")).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.code, "internal");
    assert!(!body.message.contains("exploded"));
    assert_eq!(body.request_id, Some(id));
}
