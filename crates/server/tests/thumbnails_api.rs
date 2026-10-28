mod common;

use std::fs;
use std::io::Cursor;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use explorer_server::http::router;
use http_body_util::BodyExt;
use image::{ImageFormat, Rgb, RgbImage};
use shared::api::error::ErrorBody;
use tempfile::TempDir;

struct Setup {
    client: Client,
    data: PathBuf,
    cache: PathBuf,
    thumbnails: std::sync::Arc<explorer_server::thumbnails::Thumbnails>,
    _dir: TempDir,
    _fake: common::fake_oidc::FakeOidc,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let (state, fake) = common::state_with_volumes(dir.path(), &["media"]).await;
    let data = state.config.data_dir.join("media");
    let cache = state.thumbnails.dir().to_owned();
    let thumbnails = std::sync::Arc::clone(&state.thumbnails);
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app, &fake).await;
    Setup {
        client,
        data,
        cache,
        thumbnails,
        _dir: dir,
        _fake: fake,
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_pixel(width, height, Rgb([200, 100, 50]));
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

const URI: &str = "/files/thumbnail?volume=media&path=Photos%2Fa.png&v=1";

fn header_of(res: &Response, name: header::HeaderName) -> String {
    res.headers()[name].to_str().unwrap().to_owned()
}

async fn body_of(res: Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

fn cached_files(s: &Setup) -> usize {
    fs::read_dir(&s.cache).unwrap().count()
}

impl Setup {
    fn write_photo(&self, bytes: &[u8]) {
        fs::create_dir_all(self.data.join("Photos")).unwrap();
        fs::write(self.data.join("Photos/a.png"), bytes).unwrap();
    }
}

#[tokio::test]
async fn thumbnail_is_served_and_cached() {
    let mut s = setup().await;
    s.write_photo(&png(640, 320));
    assert_eq!(cached_files(&s), 0);

    let first = s.client.get(URI).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(header_of(&first, header::CONTENT_TYPE), "image/jpeg");
    assert_eq!(
        header_of(&first, header::CACHE_CONTROL),
        "private, max-age=604800"
    );
    let etag = header_of(&first, header::ETAG);
    let first_body = body_of(first).await;
    let decoded = image::load_from_memory(&first_body).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (320, 160));
    assert_eq!(cached_files(&s), 1);

    let second = s.client.get(URI).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(header_of(&second, header::ETAG), etag);
    assert_eq!(body_of(second).await, first_body);
    assert_eq!(cached_files(&s), 1);
}

/// A request queued behind the render permits holds no descriptor of its source: the file is
/// opened once a permit is granted, so one deleted meanwhile has no thumbnail.
#[tokio::test]
async fn queued_request_opens_its_source_only_once_it_may_render() {
    let mut s = setup().await;
    s.write_photo(&png(64, 64));
    let held = [
        s.thumbnails.permit().await.unwrap(),
        s.thumbnails.permit().await.unwrap(),
    ];
    let mut request = Box::pin(s.client.get(URI));
    let waited = tokio::time::timeout(std::time::Duration::from_millis(300), &mut request).await;
    assert!(waited.is_err(), "the request did not wait for a permit");
    fs::remove_file(s.data.join("Photos/a.png")).unwrap();
    drop(held);
    let res = request.await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn if_none_match_gives_304() {
    let mut s = setup().await;
    s.write_photo(&png(40, 40));
    let etag = header_of(&s.client.get(URI).await, header::ETAG);
    assert!(etag.starts_with('"') && etag.ends_with('"'));

    let req = Request::get(URI)
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header_of(&res, header::ETAG), etag);
    assert!(body_of(res).await.is_empty());

    let req = Request::get(URI)
        .header(header::IF_NONE_MATCH, "\"something-else\"")
        .body(Body::empty())
        .unwrap();
    assert_eq!(s.client.send(req).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn changed_file_gets_a_new_thumbnail() {
    let mut s = setup().await;
    s.write_photo(&png(40, 40));
    let before = header_of(&s.client.get(URI).await, header::ETAG);
    s.write_photo(&png(80, 40));
    let res = s.client.get(URI).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_ne!(header_of(&res, header::ETAG), before);
    let decoded = image::load_from_memory(&body_of(res).await).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (80, 40));
}

async fn unavailable(client: &mut Client, uri: &str) {
    let res = client.get(uri).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "thumbnail_unavailable", "{uri}");
}

#[tokio::test]
async fn unsupported_or_missing_is_404() {
    let mut s = setup().await;
    fs::write(s.data.join("notes.txt"), png(10, 10)).unwrap();
    fs::create_dir(s.data.join("Album.jpg")).unwrap();
    fs::write(s.data.join("fake.jpg"), b"plain text, not a picture").unwrap();
    for uri in [
        "/files/thumbnail?volume=media&path=notes.txt",
        "/files/thumbnail?volume=media&path=Album.jpg",
        "/files/thumbnail?volume=media&path=missing.jpg",
        "/files/thumbnail?volume=media&path=fake.jpg",
        "/files/thumbnail?volume=media&path=",
    ] {
        unavailable(&mut s.client, uri).await;
    }
    assert_eq!(cached_files(&s), 0);
}

#[tokio::test]
async fn thumbnail_headers_are_safe() {
    let mut s = setup().await;
    s.write_photo(&png(40, 40));
    let res = s.client.get(URI).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(header_of(&res, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(header_of(&res, header::CONTENT_SECURITY_POLICY), "sandbox");
    assert!(res.headers().get(header::CONTENT_DISPOSITION).is_none());
}

#[tokio::test]
async fn thumbnails_require_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let res = Client::new(app).get(URI).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "unauthenticated");
}

#[tokio::test]
async fn unreadable_source_is_permission_denied() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup().await;
    s.write_photo(&png(64, 64));
    let photo = s.data.join("Photos/a.png");
    fs::set_permissions(&photo, fs::Permissions::from_mode(0o000)).unwrap();

    let res = s.client.get(URI).await;

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "permission_denied");
    assert_eq!(cached_files(&s), 0);
}
