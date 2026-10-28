mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use common::fake_oidc::{FakeOidc, SUBJECT};
use explorer_server::favorites::{Added, Favorites, MAX_FAVORITES};
use explorer_server::http::router;
use serde_json::json;
use shared::api::error::ErrorBody;
use shared::api::favorites::Favorite;
use tempfile::TempDir;

const ORIGIN: &str = "https://files.example.com";

struct Setup {
    client: Client,
    store: Arc<Favorites>,
    data: PathBuf,
    _fake: FakeOidc,
    _dir: TempDir,
}

async fn setup() -> Setup {
    setup_with(|_| {}).await
}

/// `prepare` runs on the data dir, with the volumes created, before they are discovered.
async fn setup_with(prepare: impl FnOnce(&Path)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    for name in ["media", "documents"] {
        fs::create_dir_all(data.join(name)).unwrap();
    }
    prepare(&data);
    let (state, fake) = common::state_with_volumes(dir.path(), &["media", "documents"]).await;
    let store = Arc::clone(&state.favorites);
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app, &fake).await;
    Setup {
        client,
        store,
        data,
        _fake: fake,
        _dir: dir,
    }
}

async fn put(client: &mut Client, volume: &str, path: &str) -> Response {
    let req = Request::put("/api/favorites")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"volume": volume, "path": path}).to_string(),
        ))
        .unwrap();
    client.send(req).await
}

async fn remove(client: &mut Client, volume: &str, path: &str) -> Response {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("volume", volume)
        .append_pair("path", path)
        .finish();
    let req = Request::delete(format!("/api/favorites?{query}"))
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    client.send(req).await
}

async fn list(client: &mut Client) -> Vec<Favorite> {
    let res = client.get("/api/favorites").await;
    assert_eq!(res.status(), StatusCode::OK);
    json(res).await
}

fn favorite(volume: &str, path: &str, exists: bool) -> Favorite {
    Favorite {
        volume: volume.to_owned(),
        path: path.to_owned(),
        exists,
    }
}

async fn error_of(res: Response, status: StatusCode, code: &str) -> ErrorBody {
    assert_eq!(res.status(), status);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, code);
    body
}

#[tokio::test]
async fn favorites_require_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let mut client = Client::new(app);
    error_of(
        client.get("/api/favorites").await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
    error_of(
        put(&mut client, "media", "").await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
    error_of(
        remove(&mut client, "media", "").await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
}

#[tokio::test]
async fn add_list_remove() {
    let mut s = setup().await;
    fs::create_dir_all(s.data.join("media/Films/2026")).unwrap();
    assert!(list(&mut s.client).await.is_empty());

    assert_eq!(
        put(&mut s.client, "media", "Films/2026").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(&mut s.client, "documents", "").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        list(&mut s.client).await,
        [
            favorite("media", "Films/2026", true),
            favorite("documents", "", true),
        ]
    );

    assert_eq!(
        remove(&mut s.client, "media", "Films/2026").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(list(&mut s.client).await, [favorite("documents", "", true)]);
    assert_eq!(
        remove(&mut s.client, "media", "Films/2026").await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn adding_twice_keeps_one() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Films")).unwrap();
    for _ in 0..2 {
        assert_eq!(
            put(&mut s.client, "media", "Films").await.status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(list(&mut s.client).await.len(), 1);
}

#[tokio::test]
async fn favorites_are_per_user() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Films")).unwrap();
    assert_eq!(
        s.store.add("someone-else", "media", "Films").await.unwrap(),
        Added::Added
    );
    assert!(list(&mut s.client).await.is_empty());

    put(&mut s.client, "media", "Films").await;
    assert_eq!(
        s.store.list(SUBJECT).await.unwrap(),
        [("media".to_owned(), "Films".to_owned())]
    );
    assert_eq!(
        s.store.list("someone-else").await.unwrap(),
        [("media".to_owned(), "Films".to_owned())]
    );

    s.store
        .remove("someone-else", "media", "Films")
        .await
        .unwrap();
    assert_eq!(list(&mut s.client).await.len(), 1);
}

#[tokio::test]
async fn add_refuses_files_and_missing() {
    let mut s = setup().await;
    fs::write(s.data.join("media/a.txt"), b"a").unwrap();
    error_of(
        put(&mut s.client, "media", "a.txt").await,
        StatusCode::BAD_REQUEST,
        "not_a_directory",
    )
    .await;
    error_of(
        put(&mut s.client, "media", "nope").await,
        StatusCode::NOT_FOUND,
        "path_not_found",
    )
    .await;
    error_of(
        put(&mut s.client, "nowhere", "").await,
        StatusCode::NOT_FOUND,
        "volume_not_found",
    )
    .await;
    error_of(
        put(&mut s.client, "media", "../x").await,
        StatusCode::BAD_REQUEST,
        "invalid_path",
    )
    .await;
    assert!(list(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn add_beyond_limit_is_409() {
    let mut s = setup().await;
    for i in 0..MAX_FAVORITES {
        let added = s
            .store
            .add(SUBJECT, "media", &format!("dir-{i}"))
            .await
            .unwrap();
        assert_eq!(added, Added::Added);
    }
    fs::create_dir(s.data.join("media/one-more")).unwrap();
    error_of(
        put(&mut s.client, "media", "one-more").await,
        StatusCode::CONFLICT,
        "favorites_full",
    )
    .await;

    // A favorite already there is not a new one.
    fs::create_dir(s.data.join("media/dir-0")).unwrap();
    assert_eq!(
        put(&mut s.client, "media", "dir-0").await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn favorites_mark_missing_folders() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir(media.join("Films")).unwrap();
    fs::create_dir(media.join("Music")).unwrap();
    fs::write(media.join("file"), b"x").unwrap();
    put(&mut s.client, "media", "Films").await;
    put(&mut s.client, "media", "Music").await;
    s.store.add(SUBJECT, "gone", "Old").await.unwrap();
    s.store.add(SUBJECT, "media", "file").await.unwrap();

    fs::rename(media.join("Films"), media.join("Movies")).unwrap();
    assert_eq!(
        list(&mut s.client).await,
        [
            favorite("media", "Films", false),
            favorite("media", "Music", true),
            favorite("gone", "Old", false),
            favorite("media", "file", false),
        ]
    );

    assert_eq!(
        remove(&mut s.client, "gone", "Old").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(list(&mut s.client).await.len(), 3);
}

#[tokio::test]
async fn favorites_work_on_read_only_volumes() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(|data| {
        fs::create_dir(data.join("documents/Taxes")).unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        put(&mut s.client, "documents", "Taxes").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(&mut s.client, "documents", "").await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        list(&mut s.client).await,
        [
            favorite("documents", "Taxes", true),
            favorite("documents", "", true),
        ]
    );
}
