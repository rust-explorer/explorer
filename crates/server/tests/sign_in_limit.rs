mod common;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use explorer_server::http::router;
use http_body_util::BodyExt;
use shared::api::error::ErrorBody;
use tower::ServiceExt;

async fn send(app: &Router, uri: &str, peer: Option<&str>, accept: Option<&str>) -> Response {
    let mut builder = Request::get(uri).header("x-request-id", "req-limit");
    if let Some(accept) = accept {
        builder = builder.header(header::ACCEPT, accept);
    }
    let mut req = builder.body(Body::empty()).unwrap();
    if let Some(peer) = peer {
        let addr: SocketAddr = peer.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
    }
    app.clone().oneshot(req).await.unwrap()
}

#[tokio::test]
async fn sign_in_routes_are_limited_per_client_address() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _fake) = common::state_with_oidc(dir.path()).await;
    let app = router(state).await.unwrap();
    let a = Some("198.51.100.1:5000");

    for _ in 0..10 {
        let res = send(&app, "/auth/login", a, None).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
    }
    // The callback shares the bucket with the login.
    let res = send(&app, "/auth/callback?code=c&state=s", a, None).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.headers()[header::RETRY_AFTER], "3");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: ErrorBody = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.code, "too_many_requests");
    assert_eq!(body.request_id.as_deref(), Some("req-limit"));

    // A browser is sent to the sign-in error page.
    let res = send(&app, "/auth/login", a, Some("text/html,*/*;q=0.8")).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().contains_key(header::RETRY_AFTER));
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let page = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        page.contains("url=/signin-error?reason=too_many_requests"),
        "{page}"
    );

    let other = send(&app, "/auth/login", Some("198.51.100.2:5000"), None).await;
    assert_eq!(other.status(), StatusCode::SEE_OTHER);
    let api = send(&app, "/api/me", a, None).await;
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn forwarded_for_is_ignored_from_an_untrusted_peer() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _fake) = common::state_with_oidc(dir.path()).await;
    let app = router(state).await.unwrap();

    for i in 0..11 {
        let mut req = common::get("/auth/login");
        req.headers_mut()
            .insert("x-forwarded-for", format!("203.0.113.{i}").parse().unwrap());
        req.extensions_mut().insert(ConnectInfo(
            "198.51.100.1:5000".parse::<SocketAddr>().unwrap(),
        ));
        let res = app.clone().oneshot(req).await.unwrap();
        let expected = if i < 10 {
            StatusCode::SEE_OTHER
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(res.status(), expected, "request {i}");
    }
}

#[tokio::test]
async fn requests_without_a_peer_address_are_not_limited() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _fake) = common::state_with_oidc(dir.path()).await;
    let app = router(state).await.unwrap();
    for _ in 0..12 {
        let res = send(&app, "/auth/login", None, None).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
    }
}

#[tokio::test]
async fn a_trusted_proxy_is_limited_per_forwarded_client() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _fake) = common::state_with_oidc(dir.path()).await;
    std::sync::Arc::get_mut(&mut state.config)
        .unwrap()
        .trusted_proxies = vec!["172.18.0.0/16".parse().unwrap()];
    let app = router(state).await.unwrap();

    let login_from = |client: &'static str| {
        let mut req = common::get("/auth/login");
        req.headers_mut().insert(
            "x-forwarded-for",
            format!("6.6.6.6, {client}").parse().unwrap(),
        );
        req.extensions_mut().insert(ConnectInfo(
            "172.18.0.2:5000".parse::<SocketAddr>().unwrap(),
        ));
        app.clone().oneshot(req)
    };
    for _ in 0..10 {
        assert_eq!(
            login_from("198.51.100.1").await.unwrap().status(),
            StatusCode::SEE_OTHER
        );
    }
    assert_eq!(
        login_from("198.51.100.1").await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login_from("198.51.100.2").await.unwrap().status(),
        StatusCode::SEE_OTHER
    );
}
