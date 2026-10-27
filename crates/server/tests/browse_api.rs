mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::client::{Client, json};
use explorer_server::http::router;
use serde_json::json;
use shared::api::browse::{EntryKind, Listing, Renamed, VolumeInfo};
use shared::api::error::ErrorBody;
use tempfile::TempDir;

struct Setup {
    client: Client,
    data: PathBuf,
    dir: TempDir,
    _fake: common::fake_oidc::FakeOidc,
}

async fn setup() -> Setup {
    setup_with(|_| {}).await
}

/// `prepare` runs on the data dir, with the volumes created, before they are discovered.
async fn setup_with(prepare: impl FnOnce(&Path)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    for name in ["media", "documents"] {
        fs::create_dir_all(dir.path().join("data").join(name)).unwrap();
    }
    prepare(&dir.path().join("data"));
    let (state, fake) = common::state_with_volumes(dir.path(), &["media", "documents"]).await;
    let data = state.config.data_dir.clone();
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app, &fake).await;
    Setup {
        client,
        data,
        dir,
        _fake: fake,
    }
}

async fn error_of(res: axum::response::Response, status: StatusCode) -> ErrorBody {
    assert_eq!(res.status(), status);
    json(res).await
}

#[tokio::test]
async fn volumes_require_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let res = Client::new(app).get("/api/volumes").await;
    let body = error_of(res, StatusCode::UNAUTHORIZED).await;
    assert_eq!(body.code, "unauthenticated");
}

#[tokio::test]
async fn lists_volumes_with_space() {
    let mut s = setup().await;
    let res = s.client.get("/api/volumes").await;
    assert_eq!(res.status(), StatusCode::OK);
    let volumes: Vec<VolumeInfo> = json(res).await;
    let names: Vec<_> = volumes.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["documents", "media"]);
    for volume in &volumes {
        assert!(!volume.read_only);
        let space = volume.space.as_ref().unwrap();
        assert!(space.total > 0 && space.available <= space.total);
    }
}

#[tokio::test]
async fn lists_root_and_subfolder() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("Films")).unwrap();
    fs::write(media.join("Films/a.txt"), b"hello").unwrap();
    fs::write(media.join("b.txt"), b"hi").unwrap();

    let res = s.client.get("/api/volumes/media/entries").await;
    assert_eq!(res.status(), StatusCode::OK);
    let root: Listing = json(res).await;
    assert_eq!((root.volume.as_str(), root.path.as_str()), ("media", ""));
    let names: Vec<_> = root.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Films", "b.txt"]);
    assert_eq!(root.entries[0].kind, EntryKind::Dir);
    assert_eq!(root.entries[1].kind, EntryKind::File);
    assert_eq!(root.entries[1].size, Some(2));
    assert!(root.entries[1].modified_ms.is_some_and(|ms| ms > 0));

    let res = s
        .client
        .get("/api/volumes/media/entries?path=Films//")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let sub: Listing = json(res).await;
    assert_eq!(sub.path, "Films");
    assert_eq!(sub.entries.len(), 1);
    assert_eq!(sub.entries[0].name, "a.txt");
    assert_eq!(sub.entries[0].size, Some(5));
}

#[tokio::test]
async fn unknown_volume_is_volume_not_found() {
    let mut s = setup().await;
    let res = s.client.get("/api/volumes/nope/entries").await;
    let body = error_of(res, StatusCode::NOT_FOUND).await;
    assert_eq!(body.code, "volume_not_found");
    assert_eq!(body.message, "Volume not found: nope");
}

#[tokio::test]
async fn missing_path_is_path_not_found_with_details() {
    let mut s = setup().await;
    let res = s.client.get("/api/volumes/media/entries?path=nope").await;
    let body = error_of(res, StatusCode::NOT_FOUND).await;
    assert_eq!(body.code, "path_not_found");
    assert_eq!(body.message, "Not found: media/nope");
    assert_eq!(
        body.details,
        Some(json!({"volume": "media", "path": "nope"}))
    );
}

#[tokio::test]
async fn file_as_folder_is_not_a_directory() {
    let mut s = setup().await;
    fs::write(s.data.join("media/a.txt"), b"x").unwrap();
    let res = s.client.get("/api/volumes/media/entries?path=a.txt").await;
    let body = error_of(res, StatusCode::BAD_REQUEST).await;
    assert_eq!(body.code, "not_a_directory");
    assert_eq!(body.message, "Not a folder: media/a.txt");
    assert_eq!(
        body.details,
        Some(json!({"volume": "media", "path": "a.txt"}))
    );
}

#[tokio::test]
async fn traversal_attempts_are_invalid_path() {
    let mut s = setup().await;
    for query in [
        "path=..",
        "path=..%2Fx",
        "path=%2e%2e/x",
        "path=a/../../x",
        "path=%2Fetc",
        "path=a%00b",
    ] {
        let res = s
            .client
            .get(&format!("/api/volumes/media/entries?{query}"))
            .await;
        let body = error_of(res, StatusCode::BAD_REQUEST).await;
        assert_eq!(body.code, "invalid_path", "{query}");
    }
    let res = s.client.get("/api/volumes/..%2F/entries").await;
    assert_ne!(res.status(), StatusCode::OK);
    let body: ErrorBody = json(res).await;
    assert!(
        ["volume_not_found", "invalid_path"].contains(&body.code.as_str()),
        "{}",
        body.code
    );
}

#[tokio::test]
async fn malformed_requests_get_a_json_invalid_path() {
    let mut s = setup().await;
    for uri in [
        "/api/volumes/media/entries?path=a&path=b",
        "/api/volumes/%FF/entries",
    ] {
        let res = s.client.get(uri).await;
        let body = error_of(res, StatusCode::BAD_REQUEST).await;
        assert_eq!(body.code, "invalid_path", "{uri}");
    }
}

#[tokio::test]
async fn special_characters_round_trip() {
    let mut s = setup().await;
    let folder = s.data.join("media/é #?%");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("f #1.txt"), b"x").unwrap();
    let res = s
        .client
        .get("/api/volumes/media/entries?path=%C3%A9%20%23%3F%25")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let listing: Listing = json(res).await;
    assert_eq!(listing.path, "é #?%");
    assert_eq!(listing.entries[0].name, "f #1.txt");
}

#[tokio::test]
async fn escaping_symlink_is_outside_volume() {
    let mut s = setup().await;
    let outside = s.dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, s.data.join("media/escape")).unwrap();
    let res = s.client.get("/api/volumes/media/entries?path=escape").await;
    let body = error_of(res, StatusCode::FORBIDDEN).await;
    assert_eq!(body.code, "outside_volume");
    assert_eq!(body.message, "media/escape points outside the volume");
    assert_eq!(
        body.details,
        Some(json!({"volume": "media", "path": "escape"}))
    );
}

async fn create_folder(s: &mut Setup, body: serde_json::Value) -> axum::response::Response {
    let req = Request::post("/api/volumes/media/folders")
        .header(header::ORIGIN, "https://files.example.com")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    s.client.send(req).await
}

#[tokio::test]
async fn exclusive_create_refuses_existing() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Films")).unwrap();
    fs::write(s.data.join("media/a.txt"), b"x").unwrap();
    for name in ["Films", "a.txt"] {
        let res = create_folder(&mut s, json!({"path": name, "exclusive": true})).await;
        let body = error_of(res, StatusCode::CONFLICT).await;
        assert_eq!(body.code, "already_exists");
    }
    assert!(s.data.join("media/a.txt").is_file());
}

#[tokio::test]
async fn exclusive_create_needs_parent() {
    let mut s = setup().await;
    let res = create_folder(&mut s, json!({"path": "missing/new", "exclusive": true})).await;
    let body = error_of(res, StatusCode::NOT_FOUND).await;
    assert_eq!(body.code, "path_not_found");
    assert!(!s.data.join("media/missing").exists());

    let res = create_folder(&mut s, json!({"path": "new", "exclusive": true})).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(s.data.join("media/new").is_dir());
}

#[tokio::test]
async fn non_exclusive_create_still_merges() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Films")).unwrap();
    let res = create_folder(&mut s, json!({"path": "Films"})).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = create_folder(&mut s, json!({"path": "a/b"})).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(s.data.join("media/a/b").is_dir());
}

async fn rename(s: &mut Setup, volume: &str, body: serde_json::Value) -> axum::response::Response {
    let req = Request::post(format!("/api/volumes/{volume}/rename"))
        .header(header::ORIGIN, "https://files.example.com")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    s.client.send(req).await
}

#[tokio::test]
async fn rename_returns_the_final_path() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir(media.join("Films")).unwrap();
    fs::write(media.join("Films/a.txt"), b"a").unwrap();
    fs::write(media.join("Films/b.txt"), b"b").unwrap();

    let res = rename(
        &mut s,
        "media",
        json!({"path": "Films/a.txt", "name": "c.txt"}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let renamed: Renamed = json(res).await;
    assert_eq!(renamed.path, "Films/c.txt");

    let res = rename(
        &mut s,
        "media",
        json!({"path": "Films/c.txt", "name": "b.txt", "conflict": "keep_both"}),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let renamed: Renamed = json(res).await;
    assert_eq!(renamed.path, "Films/b (1).txt");
    assert_eq!(fs::read(media.join("Films/b (1).txt")).unwrap(), b"a");
}

#[tokio::test]
async fn rename_onto_existing_is_conflict() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::write(media.join("a.txt"), b"a").unwrap();
    fs::write(media.join("b.txt"), b"b").unwrap();

    let res = rename(&mut s, "media", json!({"path": "a.txt", "name": "b.txt"})).await;
    let body = error_of(res, StatusCode::CONFLICT).await;
    assert_eq!(body.code, "already_exists");
    assert_eq!(
        body.details,
        Some(json!({"volume": "media", "path": "b.txt"}))
    );
    assert_eq!(fs::read(media.join("a.txt")).unwrap(), b"a");
}

#[tokio::test]
async fn rename_on_read_only_volume_is_forbidden() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(|data| {
        fs::write(data.join("documents/a.txt"), b"a").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    let res = rename(
        &mut s,
        "documents",
        json!({"path": "a.txt", "name": "b.txt"}),
    )
    .await;
    let body = error_of(res, StatusCode::FORBIDDEN).await;
    assert_eq!(body.code, "read_only_volume");
    assert!(s.data.join("documents/a.txt").is_file());
}

#[tokio::test]
async fn rename_to_a_path_is_invalid() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir(media.join("dir")).unwrap();
    fs::write(media.join("a.txt"), b"a").unwrap();

    for body in [
        json!({"path": "a.txt", "name": "dir/b.txt"}),
        json!({"path": "a.txt", "name": ".."}),
        json!({"path": "", "name": "x"}),
        json!({"path": "a.txt", "name": ".explorer-uploads"}),
    ] {
        let res = rename(&mut s, "media", body.clone()).await;
        let error = error_of(res, StatusCode::BAD_REQUEST).await;
        assert_eq!(error.code, "invalid_path", "{body}");
    }
    assert!(media.join("a.txt").is_file());
    assert_eq!(fs::read_dir(media.join("dir")).unwrap().count(), 0);
}
