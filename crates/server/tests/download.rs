mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use explorer_server::app::router;
use http_body_util::BodyExt;
use shared::api::ErrorBody;
use tempfile::TempDir;

const CONTENT: &[u8] = b"0123456789abcdef";

struct Setup {
    client: Client,
    data: PathBuf,
    dir: TempDir,
    _fake: common::fake_oidc::FakeOidc,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let (state, fake) = common::state_with_volumes(dir.path(), &["media"]).await;
    let data = state.config.data_dir.clone();
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app, &fake).await;
    fs::write(data.join("media/a.bin"), CONTENT).unwrap();
    Setup {
        client,
        data,
        dir,
        _fake: fake,
    }
}

fn header_of(res: &Response, name: impl header::AsHeaderName) -> &str {
    res.headers()[name].to_str().unwrap()
}

async fn body_of(res: Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

async fn get_with(
    client: &mut Client,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> Response {
    let mut req = Request::get(uri);
    for (name, value) in headers {
        req = req.header(name, *value);
    }
    client.send(req.body(Body::empty()).unwrap()).await
}

async fn ranged(client: &mut Client, range: &str) -> Response {
    get_with(
        client,
        "/files/download?volume=media&path=a.bin",
        &[(header::RANGE, range)],
    )
    .await
}

fn assert_attachment(res: &Response) {
    assert_eq!(
        header_of(res, header::CONTENT_TYPE),
        "application/octet-stream"
    );
    assert_eq!(header_of(res, header::CONTENT_SECURITY_POLICY), "sandbox");
    assert_eq!(header_of(res, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert!(header_of(res, header::CONTENT_DISPOSITION).starts_with("attachment"));
}

async fn error_of(res: Response, status: StatusCode) -> ErrorBody {
    assert_eq!(res.status(), status);
    json(res).await
}

#[tokio::test]
async fn downloads_whole_file_as_attachment() {
    let mut s = setup().await;
    let res = s
        .client
        .get("/files/download?volume=media&path=a.bin")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_attachment(&res);
    assert_eq!(header_of(&res, header::ACCEPT_RANGES), "bytes");
    assert_eq!(header_of(&res, header::CONTENT_LENGTH), "16");
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"a.bin\""));
    assert!(header_of(&res, header::ETAG).starts_with("W/\"16-"));
    assert!(res.headers().contains_key(header::LAST_MODIFIED));
    assert_eq!(body_of(res).await, CONTENT);
}

#[tokio::test]
async fn html_file_is_never_inline() {
    let mut s = setup().await;
    fs::write(s.data.join("media/x.html"), b"<script>alert(1)</script>").unwrap();
    let res = s
        .client
        .get("/files/download?volume=media&path=x.html")
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_attachment(&res);
}

#[tokio::test]
async fn range_from_offset_is_partial() {
    let mut s = setup().await;
    let res = ranged(&mut s.client, "bytes=5-").await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header_of(&res, header::CONTENT_RANGE), "bytes 5-15/16");
    assert_eq!(header_of(&res, header::CONTENT_LENGTH), "11");
    assert_eq!(body_of(res).await, &CONTENT[5..]);
}

#[tokio::test]
async fn suffix_and_bounded_ranges() {
    let mut s = setup().await;
    let res = ranged(&mut s.client, "bytes=-3").await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header_of(&res, header::CONTENT_RANGE), "bytes 13-15/16");
    assert_eq!(body_of(res).await, b"def");

    let res = ranged(&mut s.client, "bytes=2-4").await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header_of(&res, header::CONTENT_RANGE), "bytes 2-4/16");
    assert_eq!(body_of(res).await, b"234");
}

#[tokio::test]
async fn multi_range_sends_full_file() {
    let mut s = setup().await;
    let res = ranged(&mut s.client, "bytes=0-1,4-5").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_of(res).await, CONTENT);
}

#[tokio::test]
async fn unsatisfiable_range_is_416() {
    let mut s = setup().await;
    let res = ranged(&mut s.client, "bytes=100-").await;
    assert_eq!(header_of(&res, header::CONTENT_RANGE), "bytes */16");
    let body = error_of(res, StatusCode::RANGE_NOT_SATISFIABLE).await;
    assert_eq!(body.code, "range_not_satisfiable");
}

#[tokio::test]
async fn if_range_mismatch_sends_full_file() {
    let mut s = setup().await;
    let res = s
        .client
        .get("/files/download?volume=media&path=a.bin")
        .await;
    let etag = header_of(&res, header::ETAG).to_owned();
    let modified = header_of(&res, header::LAST_MODIFIED).to_owned();
    let uri = "/files/download?volume=media&path=a.bin";

    let res = get_with(
        &mut s.client,
        uri,
        &[
            (header::RANGE, "bytes=5-"),
            (header::IF_RANGE, "W/\"16-0\""),
        ],
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_of(res).await, CONTENT);

    let res = get_with(
        &mut s.client,
        uri,
        &[
            (header::RANGE, "bytes=5-"),
            (header::IF_RANGE, "Thu, 01 Jan 1970 00:00:00 GMT"),
        ],
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    for validator in [etag, modified] {
        let res = get_with(
            &mut s.client,
            uri,
            &[(header::RANGE, "bytes=5-"), (header::IF_RANGE, &validator)],
        )
        .await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT, "{validator}");
    }
}

#[tokio::test]
async fn head_has_headers_and_no_body() {
    let mut s = setup().await;
    let req = Request::head("/files/download?volume=media&path=a.bin")
        .body(Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_attachment(&res);
    assert_eq!(header_of(&res, header::CONTENT_LENGTH), "16");
    assert_eq!(header_of(&res, header::ACCEPT_RANGES), "bytes");
    assert!(res.headers().contains_key(header::ETAG));
    assert!(body_of(res).await.is_empty());
}

#[tokio::test]
async fn folder_is_is_a_directory() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Films")).unwrap();
    let res = s
        .client
        .get("/files/download?volume=media&path=Films")
        .await;
    let body = error_of(res, StatusCode::BAD_REQUEST).await;
    assert_eq!(body.code, "is_a_directory");
}

#[tokio::test]
async fn missing_file_and_volume() {
    let mut s = setup().await;
    let res = s.client.get("/files/download?volume=media&path=nope").await;
    assert_eq!(
        error_of(res, StatusCode::NOT_FOUND).await.code,
        "path_not_found"
    );
    let res = s.client.get("/files/download?volume=nope&path=a.bin").await;
    assert_eq!(
        error_of(res, StatusCode::NOT_FOUND).await.code,
        "volume_not_found"
    );
}

#[tokio::test]
async fn traversal_is_invalid_path() {
    let mut s = setup().await;
    let res = s
        .client
        .get("/files/download?volume=media&path=../a.bin")
        .await;
    let body = error_of(res, StatusCode::BAD_REQUEST).await;
    assert_eq!(body.code, "invalid_path");
}

#[tokio::test]
async fn escaping_symlink_is_outside_volume() {
    let mut s = setup().await;
    let outside = s.dir.path().join("secret.txt");
    fs::write(&outside, b"secret").unwrap();
    symlink(&outside, s.data.join("media/escape")).unwrap();
    let res = s
        .client
        .get("/files/download?volume=media&path=escape")
        .await;
    let body = error_of(res, StatusCode::FORBIDDEN).await;
    assert_eq!(body.code, "outside_volume");
}

#[tokio::test]
async fn download_requires_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let res = Client::new(app)
        .get("/files/download?volume=media&path=a.bin")
        .await;
    let body = error_of(res, StatusCode::UNAUTHORIZED).await;
    assert_eq!(body.code, "unauthenticated");
}
