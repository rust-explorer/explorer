mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use axum::http::{StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use explorer_server::http::router;
use http_body_util::BodyExt;
use shared::api::error::ErrorBody;
use tempfile::TempDir;
use zip::{CompressionMethod, ZipArchive};

/// 2023-11-14 22:13:20 UTC, an even second so the DOS timestamp is exact.
const MTIME_SECS: u64 = 1_700_000_000;

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
    Setup {
        client,
        data,
        dir,
        _fake: fake,
    }
}

fn write_file(path: &std::path::Path, content: &[u8]) {
    fs::write(path, content).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(MTIME_SECS))
        .unwrap();
}

fn header_of(res: &Response, name: impl header::AsHeaderName) -> &str {
    res.headers()[name].to_str().unwrap()
}

async fn archive_of(res: Response) -> ZipArchive<Cursor<Vec<u8>>> {
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    ZipArchive::new(Cursor::new(body)).unwrap()
}

/// Entry name to its content, or `None` for a directory.
fn entries(archive: &mut ZipArchive<Cursor<Vec<u8>>>) -> BTreeMap<String, Option<Vec<u8>>> {
    (0..archive.len())
        .map(|i| {
            let mut entry = archive.by_index(i).unwrap();
            assert_eq!(entry.compression(), CompressionMethod::Stored);
            let content = if entry.is_dir() {
                None
            } else {
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                Some(content)
            };
            (entry.name().unwrap().into_owned(), content)
        })
        .collect()
}

async fn error_of(res: Response, status: StatusCode) -> ErrorBody {
    assert_eq!(res.status(), status);
    json(res).await
}

#[tokio::test]
async fn zips_folder_tree() {
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir_all(docs.join("sub")).unwrap();
    fs::create_dir(docs.join("empty")).unwrap();
    write_file(&docs.join("a.txt"), b"alpha");
    write_file(&docs.join("sub/b.txt"), b"beta");
    write_file(&docs.join("Été.txt"), b"");
    write_file(&s.data.join("media/outside.txt"), b"not in the archive");

    let res = s.client.get("/files/zip?volume=media&path=Docs").await;
    assert_eq!(
        header_of(&res, header::CONTENT_TYPE),
        "application/octet-stream"
    );
    assert_eq!(header_of(&res, header::CONTENT_SECURITY_POLICY), "sandbox");
    assert_eq!(header_of(&res, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"Docs.zip\""));
    assert!(res.headers().get(header::CONTENT_LENGTH).is_none());

    let mut archive = archive_of(res).await;
    let expected: BTreeMap<String, Option<Vec<u8>>> = [
        ("a.txt", Some(&b"alpha"[..])),
        ("empty/", None),
        ("sub/", None),
        ("sub/b.txt", Some(&b"beta"[..])),
        ("Été.txt", Some(&b""[..])),
    ]
    .into_iter()
    .map(|(name, content)| (name.to_owned(), content.map(<[u8]>::to_vec)))
    .collect();
    assert_eq!(entries(&mut archive), expected);

    let modified = archive
        .by_name("sub/b.txt")
        .unwrap()
        .last_modified()
        .unwrap();
    assert_eq!(
        (
            modified.year(),
            modified.month(),
            modified.day(),
            modified.hour(),
            modified.minute(),
            modified.second()
        ),
        (2023, 11, 14, 22, 13, 20)
    );
}

#[tokio::test]
async fn zips_volume_root_named_after_volume() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a.txt"), b"alpha");
    let res = s.client.get("/files/zip?volume=media&path=").await;
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"media.zip\""));
    let mut archive = archive_of(res).await;
    assert_eq!(
        entries(&mut archive),
        BTreeMap::from([("a.txt".to_owned(), Some(b"alpha".to_vec()))])
    );
}

#[tokio::test]
async fn escaping_symlink_is_not_followed() {
    let mut s = setup().await;
    let secret_dir = s.dir.path().join("secret");
    fs::create_dir(&secret_dir).unwrap();
    fs::write(secret_dir.join("key.txt"), b"top-secret-bytes").unwrap();
    symlink(secret_dir.join("key.txt"), s.data.join("media/escape.txt")).unwrap();
    symlink(&secret_dir, s.data.join("media/escape-dir")).unwrap();
    write_file(&s.data.join("media/a.txt"), b"alpha");

    let res = s.client.get("/files/zip?volume=media&path=").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    assert!(!body.windows(16).any(|w| w == b"top-secret-bytes"));
    let mut archive = ZipArchive::new(Cursor::new(body)).unwrap();
    assert_eq!(
        entries(&mut archive).into_keys().collect::<Vec<_>>(),
        ["a.txt"]
    );
}

#[tokio::test]
async fn missing_folder_is_json_error_before_streaming() {
    let mut s = setup().await;
    let res = s.client.get("/files/zip?volume=media&path=nope").await;
    let body = error_of(res, StatusCode::NOT_FOUND).await;
    assert_eq!(body.code, "path_not_found");
    let res = s.client.get("/files/zip?volume=nope&path=").await;
    assert_eq!(
        error_of(res, StatusCode::NOT_FOUND).await.code,
        "volume_not_found"
    );
}

#[tokio::test]
async fn unreadable_subfolder_is_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup().await;
    let locked = s.data.join("media/Docs/locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let res = s.client.get("/files/zip?volume=media&path=Docs").await;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let body = error_of(res, StatusCode::FORBIDDEN).await;
    assert_eq!(body.code, "permission_denied");
    assert!(
        body.message.contains("media/Docs/locked"),
        "{}",
        body.message
    );
}

#[tokio::test]
async fn unreadable_file_mid_stream_fails_the_body() {
    use std::os::unix::fs::PermissionsExt;

    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir(&docs).unwrap();
    write_file(&docs.join("a.txt"), b"alpha");
    write_file(&docs.join("b.txt"), b"beta");
    fs::set_permissions(docs.join("b.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let res = s.client.get("/files/zip?volume=media&path=Docs").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.into_body().collect().await.is_err());
}

#[tokio::test]
async fn zip_requires_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let res = Client::new(app).get("/files/zip?volume=media&path=").await;
    let body = error_of(res, StatusCode::UNAUTHORIZED).await;
    assert_eq!(body.code, "unauthenticated");
}

#[tokio::test]
async fn traversal_is_invalid_path() {
    let mut s = setup().await;
    let res = s.client.get("/files/zip?volume=media&path=../data").await;
    let body = error_of(res, StatusCode::BAD_REQUEST).await;
    assert_eq!(body.code, "invalid_path");
}

#[tokio::test]
async fn head_is_not_allowed() {
    let mut s = setup().await;
    let req = axum::http::Request::builder()
        .method("HEAD")
        .uri("/files/zip?volume=media&path=")
        .body(axum::body::Body::empty())
        .unwrap();
    let res = s.client.send(req).await;
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(header_of(&res, header::ALLOW), "GET");
}

const HEAD: usize = 1 << 20;
const TAIL: usize = 1 << 20;

/// The streamed archive with its long run of zero bytes elided: reads as the original.
struct Elided {
    head: Vec<u8>,
    tail: Vec<u8>,
    len: u64,
    pos: u64,
}

impl Read for Elided {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let tail_start = self.len - self.tail.len() as u64;
        let n = if self.pos >= self.len {
            0
        } else if self.pos < self.head.len() as u64 {
            let from = usize::try_from(self.pos).unwrap();
            let n = buf.len().min(self.head.len() - from);
            buf[..n].copy_from_slice(&self.head[from..from + n]);
            n
        } else if self.pos < tail_start {
            let n = buf
                .len()
                .min(usize::try_from(tail_start - self.pos).unwrap());
            buf[..n].fill(0);
            n
        } else {
            let from = usize::try_from(self.pos - tail_start).unwrap();
            let n = buf.len().min(self.tail.len() - from);
            buf[..n].copy_from_slice(&self.tail[from..from + n]);
            n
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Elided {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::End(n) => self.len.checked_add_signed(n).unwrap(),
            SeekFrom::Current(n) => self.pos.checked_add_signed(n).unwrap(),
        };
        Ok(self.pos)
    }
}

#[tokio::test]
#[ignore = "streams a 4.5 GiB archive; run in release"]
async fn zips_file_over_4gib() {
    const SIZE: u64 = 4 * (1 << 30) + (1 << 29);
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/Big")).unwrap();
    fs::File::create(s.data.join("media/Big/sparse.bin"))
        .unwrap()
        .set_len(SIZE)
        .unwrap();

    let res = s.client.get("/files/zip?volume=media&path=Big").await;
    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body();
    let (mut head, mut tail, mut len) = (Vec::new(), Vec::new(), 0u64);
    while let Some(frame) = body.frame().await {
        let Ok(chunk) = frame.unwrap().into_data() else {
            continue;
        };
        let mut chunk = &chunk[..];
        let take = chunk.len().min(HEAD - head.len());
        head.extend_from_slice(&chunk[..take]);
        len += take as u64;
        chunk = &chunk[take..];
        len += chunk.len() as u64;
        tail.extend_from_slice(chunk);
        if tail.len() > 2 * TAIL {
            let excess = tail.len() - TAIL;
            assert!(tail[..excess].iter().all(|&b| b == 0), "non-zero file data");
            tail.drain(..excess);
        }
    }
    assert!(len > SIZE);

    let mut archive = ZipArchive::new(Elided {
        head,
        tail,
        len,
        pos: 0,
    })
    .unwrap();
    assert_eq!(archive.len(), 1);
    let mut entry = archive.by_name("sparse.bin").unwrap();
    assert_eq!(entry.size(), SIZE);
    assert_eq!(entry.compression(), CompressionMethod::Stored);
    // Reading to the end checks the CRC stored in the archive against the data.
    assert_eq!(io::copy(&mut entry, &mut io::sink()).unwrap(), SIZE);
}
