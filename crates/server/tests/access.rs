mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use explorer_server::auth::CurrentUser;
use explorer_server::authz::Action;
use explorer_server::http::access::authorize;
use explorer_server::http::error::ApiError;
use shared::api::ErrorBody;

async fn failure(err: ApiError) -> (StatusCode, ErrorBody) {
    let res = err.into_response();
    (res.status(), common::client::json(res).await)
}

fn user() -> CurrentUser {
    CurrentUser {
        sub: "sub".to_owned(),
        name: "User".to_owned(),
        email: None,
        groups: vec![common::ALLOWED_GROUP.to_owned()],
    }
}

#[tokio::test]
async fn authorize_checks_in_order() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("data/documents");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let (state, _fake) = common::state_with_volumes(dir.path(), &["media", "documents"]).await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let user = user();

    let err = authorize(&state, &user, "nope", "../x", Action::Write).unwrap_err();
    let (status, body) = failure(err).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body.code, "volume_not_found");

    let err = authorize(&state, &user, "media", "../x", Action::Read).unwrap_err();
    let (status, body) = failure(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.code, "invalid_path");

    let err = authorize(&state, &user, "documents", "../x", Action::Write).unwrap_err();
    let (status, body) = failure(err).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body.code, "read_only_volume");
    assert_eq!(body.details.unwrap()["volume"], "documents");

    let path = authorize(&state, &user, "documents", "a/b", Action::Read).unwrap();
    assert_eq!(path.as_str(), "a/b");
    assert!(authorize(&state, &user, "media", "a", Action::Write).is_ok());
}
