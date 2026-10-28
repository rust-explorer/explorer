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
use explorer_server::downloads::{SKIPPED_LIST, ZIP_CONCURRENCY, ZIP_CONCURRENCY_PER_USER};
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
    fake: common::fake_oidc::FakeOidc,
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
        fake,
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
    let entries = entries(&mut archive);
    assert_eq!(entries.keys().collect::<Vec<_>>(), ["a.txt", SKIPPED_LIST]);
    let list = String::from_utf8(entries[SKIPPED_LIST].clone().unwrap()).unwrap();
    assert!(
        list.contains("escape.txt: symbolic link that is broken"),
        "{list}"
    );
    assert!(
        list.contains("escape-dir: symbolic link that is broken"),
        "{list}"
    );
    assert!(!list.contains("secret"), "{list}");
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

/// The zip goes on without what it cannot read, and lists it.
#[tokio::test]
async fn unreadable_entries_are_listed_not_fatal() {
    use std::os::unix::fs::PermissionsExt;

    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir_all(docs.join("locked")).unwrap();
    write_file(&docs.join("a.txt"), b"alpha");
    write_file(&docs.join("b.txt"), b"beta");
    fs::set_permissions(docs.join("b.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(docs.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let res = s.client.get("/files/zip?volume=media&path=Docs").await;
    let mut archive = archive_of(res).await;
    let locked = s
        .client
        .get("/files/zip?volume=media&path=Docs/locked")
        .await;
    fs::set_permissions(docs.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();

    let entries = entries(&mut archive);
    assert_eq!(entries.keys().collect::<Vec<_>>(), ["a.txt", SKIPPED_LIST]);
    let list = String::from_utf8(entries[SKIPPED_LIST].clone().unwrap()).unwrap();
    assert!(list.contains("b.txt: cannot be read"), "{list}");
    assert!(list.contains("locked: cannot be read"), "{list}");

    let body = error_of(locked, StatusCode::FORBIDDEN).await;
    assert_eq!(body.code, "permission_denied");
}

/// Names that are no valid path for the API are archived all the same, made safe.
#[tokio::test]
async fn odd_names_are_archived_under_safe_names() {
    use std::os::unix::ffi::OsStrExt;

    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir(&docs).unwrap();
    write_file(&docs.join("line\nbreak.txt"), b"one");
    write_file(&docs.join("line_break.txt"), b"two");
    write_file(&docs.join("tab\there"), b"three");
    // APFS refuses names that are not UTF-8; ext4 takes them.
    let latin1 = docs.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
    let has_latin1 = fs::write(&latin1, b"four").is_ok();

    let res = s.client.get("/files/zip?volume=media&path=Docs").await;
    let mut archive = archive_of(res).await;
    let entries = entries(&mut archive);
    let mut expected = BTreeMap::from([
        ("line_break (2).txt".to_owned(), Some(b"two".to_vec())),
        ("line_break.txt".to_owned(), Some(b"one".to_vec())),
        ("tab_here".to_owned(), Some(b"three".to_vec())),
    ]);
    if has_latin1 {
        expected.insert("caf\u{fffd}.txt".to_owned(), Some(b"four".to_vec()));
    }
    assert_eq!(entries, expected);
}

/// Past 4096 bytes of path, which no API path can name, the zip still goes down.
#[tokio::test]
async fn deep_paths_are_archived() {
    use rustix::fs::{Mode, OFlags};

    let mut s = setup().await;
    let component = "d".repeat(250);
    let mut fd = rustix::fs::open(
        s.data.join("media"),
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .unwrap();
    for _ in 0..70 {
        rustix::fs::mkdirat(&fd, component.as_str(), Mode::from_raw_mode(0o755)).unwrap();
        fd = rustix::fs::openat(
            &fd,
            component.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .unwrap();
    }
    let file = rustix::fs::openat(
        &fd,
        "deep.txt",
        OFlags::WRONLY | OFlags::CREATE,
        Mode::from_raw_mode(0o644),
    )
    .unwrap();
    rustix::io::write(&file, b"down here").unwrap();

    let res = s.client.get("/files/zip?volume=media&path=").await;
    let mut archive = archive_of(res).await;
    let entries = entries(&mut archive);
    let deep = format!("{}deep.txt", format!("{component}/").repeat(70));
    assert_eq!(entries.len(), 71);
    assert_eq!(entries[&deep], Some(b"down here".to_vec()));
}

#[tokio::test]
async fn zip_of_selected_names() {
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir_all(docs.join("pics/sub")).unwrap();
    fs::create_dir(docs.join("other")).unwrap();
    write_file(&docs.join("a.txt"), b"alpha");
    write_file(&docs.join("b.txt"), b"beta");
    write_file(&docs.join("pics/p.png"), b"png");
    write_file(&docs.join("pics/sub/q.png"), b"qng");
    write_file(&docs.join("other/skipped.txt"), b"no");
    write_file(&docs.join("unselected.txt"), b"no");

    let res = s
        .client
        .get("/files/zip?volume=media&path=Docs&name=a.txt&name=pics&name=b.txt")
        .await;
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"Docs.zip\""));
    let mut archive = archive_of(res).await;
    let expected: BTreeMap<String, Option<Vec<u8>>> = [
        ("a.txt", Some(&b"alpha"[..])),
        ("b.txt", Some(&b"beta"[..])),
        ("pics/", None),
        ("pics/p.png", Some(&b"png"[..])),
        ("pics/sub/", None),
        ("pics/sub/q.png", Some(&b"qng"[..])),
    ]
    .into_iter()
    .map(|(name, content)| (name.to_owned(), content.map(<[u8]>::to_vec)))
    .collect();
    assert_eq!(entries(&mut archive), expected);
}

#[tokio::test]
async fn zip_of_selected_names_at_the_root_is_named_after_the_volume() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a%b.txt"), b"alpha");
    let res = s
        .client
        .get("/files/zip?volume=media&path=&name=a%25b.txt")
        .await;
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"media.zip\""));
    let mut archive = archive_of(res).await;
    assert_eq!(
        entries(&mut archive),
        BTreeMap::from([("a%b.txt".to_owned(), Some(b"alpha".to_vec()))])
    );
}

#[tokio::test]
async fn zip_selection_rejects_bad_names() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a.txt"), b"alpha");
    for (name, status, code) in [
        ("a%2Fb", StatusCode::BAD_REQUEST, "invalid_path"),
        ("..", StatusCode::BAD_REQUEST, "invalid_path"),
        (".", StatusCode::BAD_REQUEST, "invalid_path"),
        ("", StatusCode::BAD_REQUEST, "invalid_path"),
        (".explorer-uploads", StatusCode::BAD_REQUEST, "invalid_path"),
        ("missing.txt", StatusCode::NOT_FOUND, "path_not_found"),
    ] {
        let uri = format!("/files/zip?volume=media&path=&name=a.txt&name={name}");
        let res = s.client.get(&uri).await;
        assert_eq!(error_of(res, status).await.code, code, "{name:?}");
    }
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

#[tokio::test]
async fn duplicate_names_are_zipped_once() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a.txt"), b"alpha");
    let res = s
        .client
        .get("/files/zip?volume=media&path=&name=a.txt&name=a.txt")
        .await;
    let mut archive = archive_of(res).await;
    assert_eq!(
        entries(&mut archive),
        BTreeMap::from([("a.txt".to_owned(), Some(b"alpha".to_vec()))])
    );
}

#[tokio::test]
async fn entries_carry_their_dates_and_an_extended_timestamp() {
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir_all(docs.join("sub")).unwrap();
    write_file(&docs.join("sub/b.txt"), b"beta");
    fs::File::open(docs.join("sub"))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(MTIME_SECS))
        .unwrap();

    let mut archive = archive_of(s.client.get("/files/zip?volume=media&path=Docs").await).await;
    for name in ["sub/", "sub/b.txt"] {
        let entry = archive.by_name(name).unwrap();
        let modified = entry.last_modified().unwrap();
        assert_eq!(
            (modified.year(), modified.month(), modified.day()),
            (2023, 11, 14),
            "{name}"
        );
        let unix = entry.extra_data_fields().find_map(|field| match field {
            zip::extra_fields::ExtraField::ExtendedTimestamp(ts) => ts.mod_time(),
            _ => None,
        });
        assert_eq!(unix, Some(u32::try_from(MTIME_SECS).unwrap()), "{name}");
    }
}

#[tokio::test]
async fn cross_site_requests_are_refused() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a.txt"), b"alpha");
    for uri in [
        "/files/zip?volume=media&path=",
        "/files/download?volume=media&path=a.txt",
    ] {
        let request = |fetch: &[(&'static str, &str)]| {
            let mut req = common::get(uri);
            for (name, value) in fetch {
                req.headers_mut().insert(*name, value.parse().unwrap());
            }
            req
        };
        for site in ["cross-site", "same-site"] {
            let req = request(&[("sec-fetch-site", site)]);
            let body = error_of(s.client.send(req).await, StatusCode::FORBIDDEN).await;
            assert_eq!(body.code, "csrf_rejected", "{uri} {site}");

            let clicked = request(&[
                ("sec-fetch-site", site),
                ("sec-fetch-mode", "navigate"),
                ("sec-fetch-user", "?1"),
            ]);
            assert_eq!(
                s.client.send(clicked).await.status(),
                StatusCode::OK,
                "{uri} {site} link"
            );
        }

        for site in ["same-origin", "none"] {
            let req = request(&[("sec-fetch-site", site)]);
            assert_eq!(
                s.client.send(req).await.status(),
                StatusCode::OK,
                "{uri} {site}"
            );
        }
        assert_eq!(s.client.send(request(&[])).await.status(), StatusCode::OK);
    }
}

/// A zip holds a slot while it streams; once all are taken, or the user's, the next one is
/// refused, and a client that leaves gives its slot back.
#[tokio::test]
async fn zips_are_limited_and_free_their_slot_when_dropped() {
    let mut s = setup().await;
    write_file(&s.data.join("media/big.bin"), &vec![7u8; 4 * 1024 * 1024]);
    let mut open = Vec::new();
    for _ in 0..ZIP_CONCURRENCY_PER_USER {
        let res = s.client.get("/files/zip?volume=media&path=").await;
        assert_eq!(res.status(), StatusCode::OK);
        open.push(res);
    }
    let res = s.client.get("/files/zip?volume=media&path=").await;
    let body = error_of(res, StatusCode::TOO_MANY_REQUESTS).await;
    assert_eq!(body.code, "too_many_zips");

    let mut user = 0;
    while open.len() < ZIP_CONCURRENCY {
        user += 1;
        s.fake
            .claims(serde_json::json!({ "sub": format!("user{user}") }));
        let mut other = Client::signed_in(s.client.app.clone(), &s.fake).await;
        let res = other.get("/files/zip?volume=media&path=").await;
        assert_eq!(res.status(), StatusCode::OK);
        open.push(res);
    }
    s.fake.claims(serde_json::json!({ "sub": "late" }));
    let mut late = Client::signed_in(s.client.app.clone(), &s.fake).await;
    let res = late.get("/files/zip?volume=media&path=").await;
    let body = error_of(res, StatusCode::TOO_MANY_REQUESTS).await;
    assert_eq!(body.code, "too_many_zips");

    drop(open.pop());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let res = late.get("/files/zip?volume=media&path=").await;
        if res.status() == StatusCode::OK {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the slot never came back"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

const ORIGIN: &str = "https://files.example.com";

async fn create_selection(client: &mut Client, body: serde_json::Value) -> Response {
    let req = axum::http::Request::post("/files/zip/selections")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    client.send(req).await
}

/// More names than a URL can carry go in a body first; the zip is then a short GET.
#[tokio::test]
async fn zip_of_a_stored_selection() {
    let mut s = setup().await;
    let docs = s.data.join("media/Docs");
    fs::create_dir(&docs).unwrap();
    let names: Vec<String> = (0..5000).map(|i| format!("IMG_2024_{i:05}.jpg")).collect();
    for name in &names {
        write_file(&docs.join(name), b"x");
    }
    write_file(&docs.join("unselected.txt"), b"no");

    let res = create_selection(
        &mut s.client,
        serde_json::json!({ "volume": "media", "path": "Docs", "names": names }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let created: shared::api::downloads::ZipSelectionCreated = json(res).await;
    let uri = format!("/files/zip?selection={}", created.id);
    let res = s.client.get(&uri).await;
    assert!(header_of(&res, header::CONTENT_DISPOSITION).contains("filename=\"Docs.zip\""));
    let mut archive = archive_of(res).await;
    let zipped = entries(&mut archive);
    assert_eq!(zipped.len(), names.len());
    assert!(!zipped.contains_key("unselected.txt"));

    let res = s.client.get("/files/zip?selection=0123").await;
    assert_eq!(
        error_of(res, StatusCode::NOT_FOUND).await.code,
        "selection_expired"
    );
}

#[tokio::test]
async fn stored_selections_are_checked_and_private() {
    let mut s = setup().await;
    write_file(&s.data.join("media/a.txt"), b"alpha");
    for (body, code) in [
        (
            serde_json::json!({ "volume": "media", "path": "", "names": [] }),
            "invalid_path",
        ),
        (
            serde_json::json!({ "volume": "media", "path": "", "names": ["a/b"] }),
            "invalid_path",
        ),
        (
            serde_json::json!({ "volume": "media", "path": "../x", "names": ["a"] }),
            "invalid_path",
        ),
        (
            serde_json::json!({ "volume": "nope", "path": "", "names": ["a"] }),
            "volume_not_found",
        ),
    ] {
        let res = create_selection(&mut s.client, body.clone()).await;
        assert!(res.status().is_client_error(), "{body}");
        assert_eq!(json::<ErrorBody>(res).await.code, code, "{body}");
    }

    let res = create_selection(
        &mut s.client,
        serde_json::json!({ "volume": "media", "path": "", "names": ["a.txt"] }),
    )
    .await;
    let created: shared::api::downloads::ZipSelectionCreated = json(res).await;
    s.fake.claims(serde_json::json!({ "sub": "other-user" }));
    let app = s.client.app.clone();
    let mut other = Client::signed_in(app, &s.fake).await;
    let res = other
        .get(&format!("/files/zip?selection={}", created.id))
        .await;
    assert_eq!(
        error_of(res, StatusCode::NOT_FOUND).await.code,
        "selection_expired"
    );
    let res = s
        .client
        .get(&format!("/files/zip?selection={}", created.id))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
}
