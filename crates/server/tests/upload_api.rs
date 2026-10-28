mod common;

use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use common::fake_oidc::FakeOidc;
use explorer_server::http::router;
use explorer_server::storage::{RelPath, StagedId};
use explorer_server::uploads::{UploadRecord, Uploads};
use http_body_util::BodyExt;
use serde_json::json;
use shared::api::conflict::ConflictPolicy;
use shared::api::error::ErrorBody;
use shared::api::uploads::{MAX_PATCHES_PER_USER, MAX_PENDING_UPLOADS, PendingUpload};
use shared::tus::encode_metadata;
use sqlx::SqlitePool;
use tempfile::TempDir;
use tokio::sync::mpsc;

const ORIGIN: &str = "https://files.example.com";
const MIB: usize = 1024 * 1024;
const OFFSET_STREAM: &str = "application/offset+octet-stream";

struct Setup {
    client: Client,
    app: Router,
    uploads: Arc<Uploads>,
    db: SqlitePool,
    data: PathBuf,
    fake: FakeOidc,
    _dir: TempDir,
}

async fn setup() -> Setup {
    setup_with(&["media"], |_| {}).await
}

/// `prepare` runs on the data dir before the volumes are discovered.
async fn setup_with(volumes: &[&str], prepare: impl FnOnce(&Path)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    for name in volumes {
        fs::create_dir_all(data.join(name)).unwrap();
    }
    prepare(&data);
    let (state, fake) = common::state_with_volumes(dir.path(), volumes).await;
    let uploads = Arc::clone(&state.uploads);
    let db = state.db.clone();
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app.clone(), &fake).await;
    Setup {
        client,
        app,
        uploads,
        db,
        data,
        fake,
        _dir: dir,
    }
}

impl Setup {
    fn media(&self) -> PathBuf {
        self.data.join("media")
    }

    fn staging(&self) -> PathBuf {
        self.media().join(".explorer-uploads")
    }
}

/// A request body fed from a channel, to hold a PATCH open or make it fail midway.
struct ChannelBody(mpsc::Receiver<io::Result<Bytes>>);

impl tokio_stream::Stream for ChannelBody {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

fn channel_body() -> (mpsc::Sender<io::Result<Bytes>>, Body) {
    let (tx, rx) = mpsc::channel(2);
    (tx, Body::from_stream(ChannelBody(rx)))
}

fn clone_client(client: &Client) -> Client {
    Client {
        app: client.app.clone(),
        jar: client.jar.clone(),
    }
}

fn id_of(location: &str) -> StagedId {
    StagedId::parse(location.rsplit('/').next().unwrap()).unwrap()
}

fn content(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
}

fn tus(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN)
        .header("tus-resumable", "1.0.0")
}

fn create_request(volume: &str, path: &str, length: u64, extra: &[(&str, &str)]) -> Request<Body> {
    let mut pairs = vec![("volume", volume), ("path", path)];
    pairs.extend_from_slice(extra);
    tus(Method::POST, "/files/uploads")
        .header("upload-length", length.to_string())
        .header("upload-metadata", encode_metadata(&pairs))
        .body(Body::empty())
        .unwrap()
}

async fn create(client: &mut Client, path: &str, length: u64, extra: &[(&str, &str)]) -> Response {
    client
        .send(create_request("media", path, length, extra))
        .await
}

/// Creates an upload on `media` and returns its location.
async fn created(client: &mut Client, path: &str, length: u64, extra: &[(&str, &str)]) -> String {
    let res = create(client, path, length, extra).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(header_of(&res, "upload-offset"), "0");
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");
    let location = header_of(&res, header::LOCATION).to_owned();
    assert!(location.starts_with("/files/uploads/"), "{location}");
    location
}

fn patch_request(location: &str, offset: usize, body: Body) -> Request<Body> {
    tus(Method::PATCH, location)
        .header(header::CONTENT_TYPE, OFFSET_STREAM)
        .header("upload-offset", offset.to_string())
        .body(body)
        .unwrap()
}

async fn patch(client: &mut Client, location: &str, offset: usize, bytes: &[u8]) -> Response {
    client
        .send(patch_request(location, offset, Body::from(bytes.to_vec())))
        .await
}

async fn head(client: &mut Client, location: &str) -> Response {
    client
        .send(tus(Method::HEAD, location).body(Body::empty()).unwrap())
        .await
}

async fn delete(client: &mut Client, location: &str) -> Response {
    client
        .send(tus(Method::DELETE, location).body(Body::empty()).unwrap())
        .await
}

fn header_of(res: &Response, name: impl header::AsHeaderName) -> &str {
    res.headers()[name].to_str().unwrap()
}

fn offset_of(res: &Response) -> u64 {
    header_of(res, "upload-offset").parse().unwrap()
}

async fn error(res: Response, status: StatusCode, code: &str) -> ErrorBody {
    assert_eq!(res.status(), status);
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, code, "{body:?}");
    body
}

/// HEAD responses carry no body.
fn assert_not_found_head(res: &Response) {
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(header_of(res, "tus-resumable"), "1.0.0");
}

async fn plain_error(res: Response, status: StatusCode, code: &str) -> ErrorBody {
    assert_eq!(res.status(), status);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, code, "{body:?}");
    body
}

fn staged_count(setup: &Setup) -> usize {
    match fs::read_dir(setup.staging()) {
        Ok(entries) => entries.count(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => 0,
        Err(err) => panic!("{err}"),
    }
}

async fn pending(client: &mut Client) -> Vec<PendingUpload> {
    let res = client.get("/api/uploads").await;
    assert_eq!(res.status(), StatusCode::OK);
    json(res).await
}

fn folder_request(volume: &str, body: &str) -> Request<Body> {
    Request::post(format!("/api/volumes/{volume}/folders"))
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn options_advertises_tus() {
    let mut s = setup().await;
    let req = Request::options("/files/uploads")
        .body(Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(header_of(&res, "tus-version"), "1.0.0");
    assert_eq!(header_of(&res, "tus-extension"), "creation,termination");
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");
}

#[tokio::test]
async fn missing_tus_resumable_is_412() {
    let mut s = setup().await;
    let req = Request::post("/files/uploads")
        .header(header::ORIGIN, ORIGIN)
        .header("upload-length", "3")
        .header(
            "upload-metadata",
            encode_metadata(&[("volume", "media"), ("path", "a.txt")]),
        )
        .body(Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(header_of(&res, "tus-version"), "1.0.0");
    error(
        res,
        StatusCode::PRECONDITION_FAILED,
        "unsupported_tus_version",
    )
    .await;

    let req = Request::head("/files/uploads/0123456789abcdef0123456789abcdef")
        .header("tus-resumable", "0.2.2")
        .body(Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(res.status(), StatusCode::PRECONDITION_FAILED);
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");
    assert_eq!(staged_count(&s), 0);
}

#[tokio::test]
async fn creation_requires_length_and_metadata() {
    let mut s = setup().await;
    let no_length = tus(Method::POST, "/files/uploads")
        .header(
            "upload-metadata",
            encode_metadata(&[("volume", "media"), ("path", "a.txt")]),
        )
        .body(Body::empty())
        .unwrap();
    error(
        s.client.send(no_length).await,
        StatusCode::BAD_REQUEST,
        "invalid_upload",
    )
    .await;

    let bad_length = tus(Method::POST, "/files/uploads")
        .header("upload-length", "-1")
        .header(
            "upload-metadata",
            encode_metadata(&[("volume", "media"), ("path", "a.txt")]),
        )
        .body(Body::empty())
        .unwrap();
    error(
        s.client.send(bad_length).await,
        StatusCode::BAD_REQUEST,
        "invalid_upload",
    )
    .await;

    for metadata in [
        String::new(),
        encode_metadata(&[("volume", "media")]),
        encode_metadata(&[("path", "a.txt")]),
        "volume bWVkaWE=,path !!".to_owned(),
        encode_metadata(&[("volume", "media"), ("path", "a.txt"), ("modified", "soon")]),
        encode_metadata(&[
            ("volume", "media"),
            ("path", "a.txt"),
            ("conflict", "merge"),
        ]),
        encode_metadata(&[
            ("volume", "media"),
            ("path", "a.txt"),
            ("fingerprint", "not hex"),
        ]),
    ] {
        let req = tus(Method::POST, "/files/uploads")
            .header("upload-length", "3")
            .header("upload-metadata", metadata.clone())
            .body(Body::empty())
            .unwrap();
        let body = error(
            s.client.send(req).await,
            StatusCode::BAD_REQUEST,
            "invalid_upload",
        )
        .await;
        assert!(!body.message.is_empty(), "{metadata}");
    }

    let unknown_volume = create_request("nope", "a.txt", 3, &[]);
    error(
        s.client.send(unknown_volume).await,
        StatusCode::NOT_FOUND,
        "volume_not_found",
    )
    .await;

    let ignored = create(&mut s.client, "a.txt", 3, &[("filetype", "text/plain")]).await;
    assert_eq!(ignored.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn uploads_in_chunks_and_commits() {
    let mut s = setup().await;
    let data = content(3 * MIB);
    let modified_ms = 1_700_000_000_123_i64;
    let location = created(
        &mut s.client,
        "Film.mkv",
        data.len() as u64,
        &[("modified", &modified_ms.to_string())],
    )
    .await;

    let res = patch(&mut s.client, &location, 0, &data[..2 * MIB]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 2 * MIB as u64);
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");

    let res = head(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(offset_of(&res), 2 * MIB as u64);
    assert_eq!(header_of(&res, "upload-length"), (3 * MIB).to_string());
    assert_eq!(header_of(&res, header::CACHE_CONTROL), "no-store");
    assert!(!s.media().join("Film.mkv").exists());

    let res = patch(&mut s.client, &location, 2 * MIB, &data[2 * MIB..]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 3 * MIB as u64);

    let path = s.media().join("Film.mkv");
    assert_eq!(fs::read(&path).unwrap(), data);
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(
        mtime.duration_since(UNIX_EPOCH).unwrap(),
        Duration::from_millis(1_700_000_000_123)
    );
    assert_eq!(staged_count(&s), 0);

    let res = head(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(offset_of(&res), 3 * MIB as u64);

    let res = patch(&mut s.client, &location, 3 * MIB, &[]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 3 * MIB as u64);
    let res = patch(&mut s.client, &location, MIB, &[]).await;
    error(res, StatusCode::CONFLICT, "offset_mismatch").await;
    assert_eq!(fs::read(&path).unwrap(), data);
    assert!(pending(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn resume_after_interrupted_patch() {
    let mut s = setup().await;
    let data = content(2 * MIB);
    let location = created(&mut s.client, "big.bin", data.len() as u64, &[]).await;

    let (tx, body) = channel_body();
    tx.send(Ok(Bytes::copy_from_slice(&data[..MIB])))
        .await
        .unwrap();
    tx.send(Err(io::Error::new(
        io::ErrorKind::ConnectionReset,
        "client gone",
    )))
    .await
    .unwrap();
    let res = s.client.send(patch_request(&location, 0, body)).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), MIB as u64);

    let res = head(&mut s.client, &location).await;
    assert_eq!(offset_of(&res), MIB as u64);

    let res = patch(&mut s.client, &location, MIB, &data[MIB..]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 2 * MIB as u64);
    assert_eq!(fs::read(s.media().join("big.bin")).unwrap(), data);
}

#[tokio::test]
async fn offset_mismatch_is_409() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 10, &[]).await;
    let res = patch(&mut s.client, &location, 5, b"abcde").await;
    let body = error(res, StatusCode::CONFLICT, "offset_mismatch").await;
    assert_eq!(body.details.unwrap()["offset"], 0);

    let req = tus(Method::PATCH, &location)
        .header(header::CONTENT_TYPE, OFFSET_STREAM)
        .body(Body::from("abc"))
        .unwrap();
    error(
        s.client.send(req).await,
        StatusCode::BAD_REQUEST,
        "invalid_upload",
    )
    .await;
    assert_eq!(offset_of(&head(&mut s.client, &location).await), 0);
}

#[tokio::test]
async fn wrong_content_type_is_415() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    let req = tus(Method::PATCH, &location)
        .header(header::CONTENT_TYPE, "application/json")
        .header("upload-offset", "0")
        .body(Body::from("abc"))
        .unwrap();
    error(
        s.client.send(req).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
    )
    .await;
    assert_eq!(offset_of(&head(&mut s.client, &location).await), 0);
}

#[tokio::test]
async fn concurrent_patch_is_423() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.bin", 8, &[]).await;

    let (tx, body) = channel_body();
    let mut first_client = clone_client(&s.client);
    let first_req = patch_request(&location, 0, body);
    let first = tokio::spawn(async move { first_client.send(first_req).await });
    tx.send(Ok(Bytes::from_static(b"abcd"))).await.unwrap();

    let mut written = 0;
    for _ in 0..200 {
        written = offset_of(&head(&mut s.client, &location).await);
        if written == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(written, 4);

    let res = patch(&mut s.client, &location, 4, b"efgh").await;
    error(res, StatusCode::LOCKED, "upload_busy").await;

    drop(tx);
    let res = first.await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 4);

    let res = patch(&mut s.client, &location, 4, b"efgh").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fs::read(s.media().join("a.bin")).unwrap(), b"abcdefgh");
}

#[tokio::test]
async fn conflict_policies() {
    let mut s = setup().await;
    fs::write(s.media().join("a.txt"), "old").unwrap();
    fs::create_dir(s.media().join("dir")).unwrap();

    let res = create(&mut s.client, "a.txt", 3, &[]).await;
    let body = error(res, StatusCode::CONFLICT, "already_exists").await;
    let details = body.details.unwrap();
    assert_eq!(details["path"], "a.txt");
    assert_eq!(details["volume"], "media");
    let res = create(&mut s.client, "a.txt", 3, &[("conflict", "fail")]).await;
    error(res, StatusCode::CONFLICT, "already_exists").await;

    let location = created(&mut s.client, "a.txt", 3, &[("conflict", "replace")]).await;
    let res = patch(&mut s.client, &location, 0, b"new").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"new");

    let location = created(&mut s.client, "a.txt", 4, &[("conflict", "keep_both")]).await;
    let res = patch(&mut s.client, &location, 0, b"both").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"new");
    assert_eq!(fs::read(s.media().join("a (1).txt")).unwrap(), b"both");

    let res = create(&mut s.client, "dir", 3, &[("conflict", "replace")]).await;
    let body = error(res, StatusCode::BAD_REQUEST, "is_a_directory").await;
    assert_eq!(body.details.unwrap()["path"], "dir");
    let res = create(&mut s.client, "a.txt/x", 3, &[("conflict", "replace")]).await;
    error(res, StatusCode::BAD_REQUEST, "not_a_directory").await;
    assert!(s.media().join("dir").is_dir());
    assert_eq!(staged_count(&s), 0);
}

#[tokio::test]
async fn folder_upload_creates_missing_dirs() {
    let mut s = setup().await;
    let location = created(&mut s.client, "Album/2026/x.jpg", 3, &[]).await;
    assert!(!s.media().join("Album").exists());
    let res = patch(&mut s.client, &location, 0, b"jpg").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        fs::read(s.media().join("Album/2026/x.jpg")).unwrap(),
        b"jpg"
    );
}

#[tokio::test]
async fn zero_length_upload_completes_on_creation() {
    let mut s = setup().await;
    let location = created(&mut s.client, "empty/e.txt", 0, &[]).await;
    assert_eq!(fs::read(s.media().join("empty/e.txt")).unwrap(), b"");
    assert_eq!(staged_count(&s), 0);

    let res = head(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(offset_of(&res), 0);
    assert_eq!(header_of(&res, "upload-length"), "0");
    assert!(pending(&mut s.client).await.is_empty());

    let res = patch(&mut s.client, &location, 0, &[]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn hostile_paths_are_rejected() {
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().to_owned();
    let mut s = setup_with(&["media"], |data| {
        symlink(&target, data.join("media/escape")).unwrap();
    })
    .await;

    for path in [
        "../x",
        "/etc/x",
        "a/../../x",
        ".explorer-uploads/x",
        "a\0b",
        "",
    ] {
        let res = create(&mut s.client, path, 3, &[]).await;
        error(res, StatusCode::BAD_REQUEST, "invalid_path").await;
    }
    let res = create(&mut s.client, "escape/x", 3, &[]).await;
    error(res, StatusCode::FORBIDDEN, "outside_volume").await;
    for conflict in ["replace", "keep_both"] {
        let res = create(&mut s.client, "escape/x", 3, &[("conflict", conflict)]).await;
        error(res, StatusCode::FORBIDDEN, "outside_volume").await;
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(staged_count(&s), 0);
    assert!(pending(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn read_only_volume_rejects_upload_and_folder() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(&["media", "documents"], |data| {
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    let res = s
        .client
        .send(create_request("documents", "a.txt", 3, &[]))
        .await;
    let body = error(res, StatusCode::FORBIDDEN, "read_only_volume").await;
    assert_eq!(body.details.unwrap()["volume"], "documents");

    let res = s
        .client
        .send(folder_request("documents", r#"{"path":"new"}"#))
        .await;
    plain_error(res, StatusCode::FORBIDDEN, "read_only_volume").await;
    assert_eq!(fs::read_dir(s.data.join("documents")).unwrap().count(), 0);
}

#[tokio::test]
async fn too_much_data_is_413() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 4, &[]).await;
    let res = patch(&mut s.client, &location, 0, b"abcdefgh").await;
    error(res, StatusCode::PAYLOAD_TOO_LARGE, "upload_too_large").await;
    assert_eq!(offset_of(&head(&mut s.client, &location).await), 4);

    let res = patch(&mut s.client, &location, 4, &[]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 4);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"abcd");
}

#[tokio::test]
async fn insufficient_space_is_507() {
    let mut s = setup().await;
    let res = create(&mut s.client, "huge.bin", 1 << 62, &[]).await;
    let body = error(
        res,
        StatusCode::INSUFFICIENT_STORAGE,
        "insufficient_storage",
    )
    .await;
    assert_eq!(body.details.unwrap()["volume"], "media");
    assert_eq!(staged_count(&s), 0);
}

#[tokio::test]
async fn other_users_upload_is_not_found() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    s.fake.claims(json!({ "sub": "other-user" }));
    let mut other = Client::signed_in(s.app.clone(), &s.fake).await;
    assert_eq!(
        json::<serde_json::Value>(other.get("/api/me").await).await["sub"],
        "other-user"
    );

    assert_not_found_head(&head(&mut other, &location).await);
    error(
        patch(&mut other, &location, 0, b"abc").await,
        StatusCode::NOT_FOUND,
        "upload_not_found",
    )
    .await;
    error(
        delete(&mut other, &location).await,
        StatusCode::NOT_FOUND,
        "upload_not_found",
    )
    .await;
    assert!(pending(&mut other).await.is_empty());

    assert_eq!(staged_count(&s), 1);
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"abc");
}

#[tokio::test]
async fn unknown_upload_is_not_found() {
    let mut s = setup().await;
    for location in [
        "/files/uploads/0123456789abcdef0123456789abcdef",
        "/files/uploads/not-an-id",
        "/files/uploads/..",
    ] {
        assert_not_found_head(&head(&mut s.client, location).await);
        error(
            patch(&mut s.client, location, 0, b"x").await,
            StatusCode::NOT_FOUND,
            "upload_not_found",
        )
        .await;
        error(
            delete(&mut s.client, location).await,
            StatusCode::NOT_FOUND,
            "upload_not_found",
        )
        .await;
    }
}

#[tokio::test]
async fn delete_cancels_and_removes_staged_file() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 10, &[]).await;
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(staged_count(&s), 1);

    let res = delete(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(header_of(&res, "tus-resumable"), "1.0.0");
    assert_eq!(staged_count(&s), 0);
    assert!(!s.media().join("a.txt").exists());
    assert_eq!(
        head(&mut s.client, &location).await.status(),
        StatusCode::NOT_FOUND
    );
    error(
        delete(&mut s.client, &location).await,
        StatusCode::NOT_FOUND,
        "upload_not_found",
    )
    .await;

    let location = created(&mut s.client, "b.txt", 1, &[]).await;
    assert_eq!(
        patch(&mut s.client, &location, 0, b"b").await.status(),
        StatusCode::NO_CONTENT
    );
    // Too late to cancel: the client must not report it as cancelled.
    let res = delete(&mut s.client, &location).await;
    let body = error(res, StatusCode::CONFLICT, "upload_committed").await;
    assert_eq!(body.details.unwrap()["final_path"], "b.txt");
    assert_eq!(fs::read(s.media().join("b.txt")).unwrap(), b"b");
    assert_eq!(
        head(&mut s.client, &location).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn pending_list_shows_unfinished_uploads_only() {
    let mut s = setup().await;
    let partial = created(
        &mut s.client,
        "Album/partial.jpg",
        10,
        &[
            ("modified", "1700000000000"),
            ("conflict", "replace"),
            ("fingerprint", "0123456789abcdef"),
        ],
    )
    .await;
    assert_eq!(
        patch(&mut s.client, &partial, 0, b"abc").await.status(),
        StatusCode::NO_CONTENT
    );
    let done = created(&mut s.client, "done.txt", 2, &[]).await;
    assert_eq!(
        patch(&mut s.client, &done, 0, b"ok").await.status(),
        StatusCode::NO_CONTENT
    );
    let orphan = created(&mut s.client, "orphan.txt", 2, &[]).await;
    let orphan_id = orphan.rsplit('/').next().unwrap();
    fs::remove_file(s.staging().join(orphan_id)).unwrap();

    let list = pending(&mut s.client).await;
    assert_eq!(
        list,
        [PendingUpload {
            id: partial.rsplit('/').next().unwrap().to_owned(),
            volume: "media".to_owned(),
            path: "Album/partial.jpg".to_owned(),
            length: 10,
            offset: 3,
            modified_ms: Some(1_700_000_000_000),
            conflict: ConflictPolicy::Replace,
            fingerprint: Some("0123456789abcdef".to_owned()),
        }]
    );

    assert_not_found_head(&head(&mut s.client, &orphan).await);
}

#[tokio::test]
async fn create_folder() {
    let mut s = setup().await;
    let res = s
        .client
        .send(folder_request("media", r#"{"path":"a/b/c"}"#))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(s.media().join("a/b/c").is_dir());
    let res = s
        .client
        .send(folder_request("media", r#"{"path":"a/b/c"}"#))
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    fs::write(s.media().join("f.txt"), "x").unwrap();
    let res = s
        .client
        .send(folder_request("media", r#"{"path":"f.txt"}"#))
        .await;
    let body = plain_error(res, StatusCode::CONFLICT, "already_exists").await;
    assert_eq!(body.details.unwrap()["path"], "f.txt");
    let res = s
        .client
        .send(folder_request("media", r#"{"path":"f.txt/x"}"#))
        .await;
    plain_error(res, StatusCode::BAD_REQUEST, "not_a_directory").await;

    for body in [
        r#"{"path":""}"#,
        r#"{"path":"/"}"#,
        r#"{"path":"../x"}"#,
        r#"{"path":".explorer-uploads"}"#,
        r#"{"nope":1}"#,
        "not json",
    ] {
        let res = s.client.send(folder_request("media", body)).await;
        plain_error(res, StatusCode::BAD_REQUEST, "invalid_path").await;
    }
    let res = s
        .client
        .send(folder_request("nope", r#"{"path":"x"}"#))
        .await;
    plain_error(res, StatusCode::NOT_FOUND, "volume_not_found").await;
    assert!(!s.media().join(".explorer-uploads").exists());
}

#[tokio::test]
async fn upload_routes_require_login() {
    let s = setup().await;
    let mut anonymous = Client::new(s.app.clone());
    let location = "/files/uploads/0123456789abcdef0123456789abcdef";
    let requests = [
        tus(Method::OPTIONS, "/files/uploads")
            .body(Body::empty())
            .unwrap(),
        create_request("media", "a.txt", 3, &[]),
        tus(Method::HEAD, location).body(Body::empty()).unwrap(),
        patch_request(location, 0, Body::from("abc")),
        tus(Method::DELETE, location).body(Body::empty()).unwrap(),
        Request::get("/api/uploads").body(Body::empty()).unwrap(),
        folder_request("media", r#"{"path":"x"}"#),
    ];
    for req in requests {
        let uri = req.uri().clone();
        let method = req.method().clone();
        let res = anonymous.send(req).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    assert!(!s.media().join("x").exists());
    assert_eq!(staged_count(&s), 0);
}

#[tokio::test]
async fn responses_outside_the_tus_router_carry_tus_resumable() {
    let mut s = setup().await;
    let res = s
        .client
        .send(
            tus(Method::GET, "/files/uploads/")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    error(res, StatusCode::NOT_FOUND, "not_found").await;

    let req = Request::post("/files/uploads")
        .header("tus-resumable", "1.0.0")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    error(
        s.client.send(req).await,
        StatusCode::FORBIDDEN,
        "csrf_rejected",
    )
    .await;

    let res = s.client.get("/files/uploadsx").await;
    assert!(res.headers().get("tus-resumable").is_none());
    let res = s.client.get("/api/uploads").await;
    assert!(res.headers().get("tus-resumable").is_none());
}

/// A PATCH holding the lock may be committing it: the staged file is gone but the record is not
/// an orphan yet.
#[tokio::test]
async fn head_keeps_a_missing_staged_file_while_locked() {
    let mut s = setup().await;
    let sub = me(&mut s.client).await;
    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    let id = id_of(&location);
    fs::remove_file(s.staging().join(id.as_str())).unwrap();

    let lock = s.uploads.try_lock(&id).unwrap();
    let res = head(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(offset_of(&res), 3);
    assert_eq!(s.uploads.list_pending(&sub).await.unwrap().len(), 1);

    drop(lock);
    assert_not_found_head(&head(&mut s.client, &location).await);
    assert!(s.uploads.list_pending(&sub).await.unwrap().is_empty());
}

async fn me(client: &mut Client) -> String {
    let me: serde_json::Value = json(client.get("/api/me").await).await;
    me["sub"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn dropped_patch_keeps_the_lock_until_its_writes_finish() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.bin", 8, &[]).await;

    let (tx, body) = channel_body();
    let mut first_client = clone_client(&s.client);
    let first_req = patch_request(&location, 0, body);
    let first = tokio::spawn(async move { first_client.send(first_req).await });
    tx.send(Ok(Bytes::from_static(b"abcd"))).await.unwrap();
    for _ in 0..200 {
        if offset_of(&head(&mut s.client, &location).await) == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());

    // The request is gone but its body is still open: the upload stays locked.
    let res = patch(&mut s.client, &location, 4, b"efgh").await;
    error(res, StatusCode::LOCKED, "upload_busy").await;
    let res = delete(&mut s.client, &location).await;
    error(res, StatusCode::LOCKED, "upload_busy").await;

    tx.send(Ok(Bytes::from_static(b"ef"))).await.unwrap();
    drop(tx);
    let mut res = patch(&mut s.client, &location, 6, b"gh").await;
    for _ in 0..200 {
        if res.status() != StatusCode::LOCKED {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        res = patch(&mut s.client, &location, 6, b"gh").await;
    }
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 8);
    assert_eq!(fs::read(s.media().join("a.bin")).unwrap(), b"abcdefgh");
}

#[tokio::test]
async fn delete_of_a_busy_upload_is_423() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    let lock = s.uploads.try_lock(&id_of(&location)).unwrap();
    let res = delete(&mut s.client, &location).await;
    error(res, StatusCode::LOCKED, "upload_busy").await;
    assert_eq!(staged_count(&s), 1);

    drop(lock);
    let res = delete(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(staged_count(&s), 0);
}

#[tokio::test]
async fn delete_during_the_last_chunk_stops_the_commit() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.bin", 8, &[]).await;

    let (tx, body) = channel_body();
    let mut first_client = clone_client(&s.client);
    let first_req = patch_request(&location, 0, body);
    let first = tokio::spawn(async move { first_client.send(first_req).await });
    tx.send(Ok(Bytes::from_static(b"abcdefgh"))).await.unwrap();
    for _ in 0..200 {
        if offset_of(&head(&mut s.client, &location).await) == 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let res = delete(&mut s.client, &location).await;
    error(res, StatusCode::LOCKED, "upload_busy").await;

    drop(tx);
    let res = first.await.unwrap();
    error(res, StatusCode::NOT_FOUND, "upload_not_found").await;
    assert!(!s.media().join("a.bin").exists());
    let res = delete(&mut s.client, &location).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(staged_count(&s), 0);
    assert!(pending(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn commit_succeeds_when_marking_it_done_fails() {
    let mut s = setup().await;
    sqlx::query(
        "CREATE TRIGGER no_done BEFORE UPDATE OF final_path ON uploads \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .execute(&s.db)
    .await
    .unwrap();

    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 3);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"abc");

    let res = create(&mut s.client, "empty.txt", 0, &[]).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert!(s.media().join("empty.txt").exists());
}

#[tokio::test]
async fn failed_commit_is_retried_by_an_empty_patch() {
    let mut s = setup().await;
    let location = created(&mut s.client, "Album/a.txt", 3, &[]).await;
    fs::write(s.media().join("Album"), "obstacle").unwrap();
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    error(res, StatusCode::CONFLICT, "already_exists").await;
    let res = head(&mut s.client, &location).await;
    assert_eq!(offset_of(&res), 3);
    assert_eq!(pending(&mut s.client).await.len(), 1);

    fs::remove_file(s.media().join("Album")).unwrap();
    let res = patch(&mut s.client, &location, 3, &[]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(offset_of(&res), 3);
    assert_eq!(fs::read(s.media().join("Album/a.txt")).unwrap(), b"abc");
    assert_eq!(staged_count(&s), 0);
    assert!(pending(&mut s.client).await.is_empty());
}

/// The last PATCH wrote every byte, but the server failed before committing.
#[tokio::test]
async fn internal_failure_after_the_last_bytes_is_retried_by_an_empty_patch() {
    let mut s = setup().await;
    let location = created(&mut s.client, "a.txt", 3, &[]).await;
    sqlx::query(
        "CREATE TRIGGER no_touch BEFORE UPDATE OF updated_at ON uploads \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .execute(&s.db)
    .await
    .unwrap();
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    error(res, StatusCode::INTERNAL_SERVER_ERROR, "internal").await;
    assert!(!s.media().join("a.txt").exists());
    let res = head(&mut s.client, &location).await;
    assert_eq!(offset_of(&res), 3);

    sqlx::query("DROP TRIGGER no_touch")
        .execute(&s.db)
        .await
        .unwrap();
    let res = patch(&mut s.client, &location, 3, &[]).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(fs::read(s.media().join("a.txt")).unwrap(), b"abc");
    assert!(pending(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn staging_is_unreachable_through_a_symlink_to_the_root() {
    let mut s = setup_with(&["media"], |data| {
        symlink(".", data.join("media/self")).unwrap();
        fs::write(data.join("media/a.txt"), "a").unwrap();
    })
    .await;
    let location = created(&mut s.client, "pending.bin", 4, &[]).await;
    let id = id_of(&location);
    patch(&mut s.client, &location, 0, b"ab").await;
    let staged = format!("self/.explorer-uploads/{}", id.as_str());

    let res = s.client.get("/api/volumes/media/entries?path=self").await;
    assert_eq!(res.status(), StatusCode::OK);
    let listing: shared::api::browse::Listing = json(res).await;
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a.txt", "self"]);

    let res = s
        .client
        .get("/api/volumes/media/entries?path=self/.explorer-uploads")
        .await;
    plain_error(res, StatusCode::NOT_FOUND, "path_not_found").await;
    let res = s
        .client
        .get(&format!("/files/download?volume=media&path={staged}"))
        .await;
    plain_error(res, StatusCode::NOT_FOUND, "path_not_found").await;

    let res = create(&mut s.client, &staged, 4, &[("conflict", "replace")]).await;
    error(res, StatusCode::NOT_FOUND, "path_not_found").await;
    let res = create(&mut s.client, "self/.explorer-uploads", 1, &[]).await;
    error(res, StatusCode::NOT_FOUND, "path_not_found").await;
    let res = s
        .client
        .send(folder_request(
            "media",
            r#"{"path":"self/.explorer-uploads/sub"}"#,
        ))
        .await;
    plain_error(res, StatusCode::NOT_FOUND, "path_not_found").await;

    let res = s.client.get("/files/zip?volume=media&path=self").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(body)).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    // The symlink to the root inside it is listed as skipped, not followed.
    assert_eq!(names, ["a.txt", "explorer-skipped.txt"]);
    let mut list = String::new();
    std::io::Read::read_to_string(
        &mut archive.by_name("explorer-skipped.txt").unwrap(),
        &mut list,
    )
    .unwrap();
    assert!(!list.contains(".explorer-uploads"), "{list}");

    assert_eq!(staged_count(&s), 1);
    assert_eq!(fs::read(s.staging().join(id.as_str())).unwrap(), b"ab");
}

#[tokio::test]
async fn unfinished_uploads_are_capped_per_user() {
    let mut s = setup().await;
    let sub = me(&mut s.client).await;
    for i in 1..MAX_PENDING_UPLOADS {
        let record = UploadRecord {
            id: StagedId::random(),
            user_sub: sub.clone(),
            volume: "media".to_owned(),
            path: RelPath::parse(&format!("f{i}")).unwrap(),
            length: 10,
            modified_ms: None,
            conflict: ConflictPolicy::Fail,
            fingerprint: None,
            final_path: None,
            created_at: 1,
            updated_at: 1,
        };
        s.uploads.insert(&record).await.unwrap();
    }

    let last = created(&mut s.client, "last.txt", 3, &[]).await;
    let res = create(&mut s.client, "over.txt", 3, &[]).await;
    error(res, StatusCode::TOO_MANY_REQUESTS, "too_many_uploads").await;

    // A finished upload no longer counts, nor does another user's.
    assert_eq!(
        patch(&mut s.client, &last, 0, b"abc").await.status(),
        StatusCode::NO_CONTENT
    );
    created(&mut s.client, "after.txt", 3, &[]).await;
    s.fake.claims(json!({ "sub": "other-user" }));
    let mut other = Client::signed_in(s.app.clone(), &s.fake).await;
    created(&mut other, "other.txt", 3, &[]).await;
}

#[tokio::test]
async fn free_space_counts_what_pending_uploads_still_need() {
    let mut s = setup().await;
    let stat = rustix::fs::statvfs(s.media()).unwrap();
    let available = stat.f_bavail * stat.f_frsize;
    let length = available / 10 * 6;

    let first = created(&mut s.client, "first.bin", length, &[]).await;
    let res = create(&mut s.client, "second.bin", length, &[]).await;
    error(
        res,
        StatusCode::INSUFFICIENT_STORAGE,
        "insufficient_storage",
    )
    .await;
    assert_eq!(staged_count(&s), 1);

    assert_eq!(
        delete(&mut s.client, &first).await.status(),
        StatusCode::NO_CONTENT
    );
    created(&mut s.client, "second.bin", length, &[]).await;
}

#[tokio::test]
async fn patches_in_flight_are_capped_per_user() {
    let mut s = setup().await;
    let mut held = Vec::new();
    for i in 0..MAX_PATCHES_PER_USER {
        let location = created(&mut s.client, &format!("held{i}.bin"), 8, &[]).await;
        let (tx, body) = channel_body();
        let mut client = clone_client(&s.client);
        let req = patch_request(&location, 0, body);
        let task = tokio::spawn(async move { client.send(req).await });
        tx.send(Ok(Bytes::from_static(b"abcd"))).await.unwrap();
        let mut written = 0;
        for _ in 0..200 {
            written = offset_of(&head(&mut s.client, &location).await);
            if written == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(written, 4);
        held.push((tx, task));
    }
    let location = created(&mut s.client, "next.bin", 3, &[]).await;

    let res = patch(&mut s.client, &location, 0, b"abc").await;
    error(res, StatusCode::TOO_MANY_REQUESTS, "too_many_patches").await;

    s.fake.claims(json!({ "sub": "other-user" }));
    let mut other = Client::signed_in(s.app.clone(), &s.fake).await;
    let theirs = created(&mut other, "theirs.bin", 3, &[]).await;
    assert_eq!(
        patch(&mut other, &theirs, 0, b"abc").await.status(),
        StatusCode::NO_CONTENT
    );

    let (tx, task) = held.pop().unwrap();
    drop(tx);
    assert_eq!(task.await.unwrap().status(), StatusCode::NO_CONTENT);
    let res = patch(&mut s.client, &location, 0, b"abc").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
}
