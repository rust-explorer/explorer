use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use explorer_server::app::{AppState, router};
use explorer_server::config::Config;
use http_body_util::BodyExt;
use shared::api::{ErrorBody, Health};
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap().to_owned();
    let config = Config::from_lookup(|key| {
        let value = match key {
            "EXPLORER_PUBLIC_URL" => "https://files.example.com",
            "EXPLORER_OIDC_ISSUER" => "https://id.example.com",
            "EXPLORER_OIDC_CLIENT_ID" => "explorer",
            "EXPLORER_OIDC_CLIENT_SECRET" => "secret",
            "EXPLORER_ALLOWED_GROUP" => "explorer-users",
            "EXPLORER_DATA_DIR" | "EXPLORER_CONFIG_DIR" | "EXPLORER_WEB_DIR" => &path,
            _ => return None,
        };
        Some(value.to_owned())
    })
    .unwrap();
    let state = AppState {
        config: Arc::new(config),
    };
    (router(state), dir)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
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
