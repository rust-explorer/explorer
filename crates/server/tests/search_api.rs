mod common;

use std::fs;

use axum::http::StatusCode;
use common::client::{Client, json};
use explorer_server::http::router;
use shared::api::browse::EntryKind;
use shared::api::error::ErrorBody;
use shared::api::search::SearchResults;

async fn setup() -> (
    Client,
    std::path::PathBuf,
    tempfile::TempDir,
    common::fake_oidc::FakeOidc,
) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["media", "documents"] {
        fs::create_dir_all(dir.path().join("data").join(name)).unwrap();
    }
    let (state, fake) = common::state_with_volumes(dir.path(), &["media", "documents"]).await;
    let data = state.config.data_dir.clone();
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app, &fake).await;
    (client, data, dir, fake)
}

#[tokio::test]
async fn search_returns_paths_and_metadata() {
    let (mut client, data, _dir, _fake) = setup().await;
    let media = data.join("media");
    fs::create_dir_all(media.join("Holidays/Photos")).unwrap();
    fs::write(media.join("Holidays/Photos/beach.JPG"), b"12345").unwrap();
    fs::write(media.join("Holidays/photo-list.txt"), b"x").unwrap();
    fs::write(media.join("unrelated.txt"), b"x").unwrap();

    let res = client.get("/api/volumes/media/search?q=%20PHOTO%20").await;
    assert_eq!(res.status(), StatusCode::OK);
    let results: SearchResults = json(res).await;
    assert_eq!(
        (results.volume.as_str(), results.query.as_str()),
        ("media", "PHOTO")
    );
    assert!(!results.truncated);
    let paths: Vec<_> = results.hits.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(paths, ["Holidays/Photos", "Holidays/photo-list.txt"]);
    assert_eq!(results.hits[1].kind, EntryKind::File);
    assert_eq!(results.hits[1].size, Some(1));
    assert!(results.hits[1].modified_ms.is_some_and(|ms| ms > 0));
    assert_eq!(results.hits[0].kind, EntryKind::Dir);
    assert_eq!(results.hits[0].size, None);
}

#[tokio::test]
async fn short_query_is_400() {
    let (mut client, _data, _dir, _fake) = setup().await;
    for uri in [
        "/api/volumes/media/search?q=a",
        "/api/volumes/media/search?q=%20a%20",
        "/api/volumes/media/search?q=",
        "/api/volumes/media/search",
    ] {
        let res = client.get(uri).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body: ErrorBody = json(res).await;
        assert_eq!(body.code, "invalid_path");
    }
}

#[tokio::test]
async fn search_on_unknown_volume_is_404() {
    let (mut client, _data, _dir, _fake) = setup().await;
    let res = client.get("/api/volumes/nope/search?q=abc").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "volume_not_found");
}

#[tokio::test]
async fn search_requires_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let res = Client::new(app)
        .get("/api/volumes/media/search?q=abc")
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "unauthenticated");
}
