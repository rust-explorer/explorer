mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use explorer_server::http::{AppState, router, router_with};
use http_body_util::BodyExt;
use tower::ServiceExt;
use tower_sessions::Session;

use common::client::Client;

const KEY: &str = "value";

async fn write(session: Session) -> &'static str {
    session.insert(KEY, "hello").await.unwrap();
    "written"
}

async fn read(session: Session) -> String {
    session
        .get::<String>(KEY)
        .await
        .unwrap()
        .unwrap_or_else(|| "none".to_owned())
}

async fn app(dir: &tempfile::TempDir) -> Router {
    let extra: Router<AppState> = Router::new()
        .route("/api/_session/write", get(write))
        .route("/api/_session/read", get(read));
    router_with(common::state_with_web_dir(dir.path()).await, extra)
        .await
        .unwrap()
}

async fn body_text(res: axum::response::Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn session_cookie(res: &axum::response::Response) -> String {
    res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn session_cookie_has_secure_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let res = app(&dir)
        .await
        .oneshot(common::get("/api/_session/write"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = session_cookie(&res);
    assert!(cookie.starts_with("explorer_session="), "{cookie}");
    for attr in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/", "Max-Age="] {
        assert!(
            cookie.split("; ").any(|a| a.starts_with(attr)),
            "{attr} missing in {cookie}"
        );
    }
}

#[tokio::test]
async fn session_survives_store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let res = app(&dir)
        .await
        .oneshot(common::get("/api/_session/write"))
        .await
        .unwrap();
    let cookie = session_cookie(&res);
    let pair = cookie.split(';').next().unwrap().to_owned();

    // A fresh router over the same database proves the value lives in the store.
    let req = Request::get("/api/_session/read")
        .header(header::COOKIE, pair)
        .body(Body::empty())
        .unwrap();
    let res = app(&dir).await.oneshot(req).await.unwrap();
    assert_eq!(body_text(res).await, "hello");
}

#[tokio::test]
async fn unknown_session_cookie_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get("/api/_session/read")
        .header(header::COOKIE, "explorer_session=AAAAAAAAAAAAAAAAAAAAAA")
        .body(Body::empty())
        .unwrap();
    let res = app(&dir).await.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // The stale cookie may only be cleared, never replaced by a fresh session.
    if let Some(cookie) = res.headers().get(header::SET_COOKIE) {
        let cookie = cookie.to_str().unwrap();
        assert!(cookie.starts_with("explorer_session=;"), "{cookie}");
        assert!(cookie.contains("Max-Age=0"), "{cookie}");
    }
    assert_eq!(body_text(res).await, "none");
}

#[tokio::test]
async fn untouched_session_sets_no_cookie() {
    let dir = tempfile::tempdir().unwrap();
    let res = app(&dir)
        .await
        .oneshot(common::get("/healthz"))
        .await
        .unwrap();
    assert!(res.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn activity_keeps_a_session_alive_past_its_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, fake) = common::state_with_oidc(dir.path()).await;
    let mut config = common::config(dir.path().to_str().unwrap(), &fake.issuer());
    config.session_ttl = Duration::from_secs(3);
    state.config = Arc::new(config);
    let mut client = Client::signed_in(router(state).await.unwrap(), &fake).await;

    let mut refreshed = 0;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let res = client.get("/api/me").await;
        assert_eq!(res.status(), StatusCode::OK);
        refreshed += usize::from(res.headers().contains_key(header::SET_COOKIE));
    }
    // Refreshed now and then, not on every request.
    assert!((1..10).contains(&refreshed), "{refreshed}");

    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(
        client.get("/api/me").await.status(),
        StatusCode::UNAUTHORIZED
    );
}
