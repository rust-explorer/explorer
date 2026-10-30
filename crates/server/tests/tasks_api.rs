mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use common::client::{Client, json};
use common::fake_oidc::FakeOidc;
use explorer_server::http::router;
use explorer_server::tasks::Tasks;
use serde_json::json;
use shared::api::error::ErrorBody;
use shared::api::tasks::{Location, TaskInfo, TaskSpec, TaskState};
use shared::api::usage::{UsageKind, UsageView};
use tempfile::TempDir;

const ORIGIN: &str = "https://files.example.com";

struct Setup {
    client: Client,
    app: Router,
    fake: FakeOidc,
    data: PathBuf,
    _dir: TempDir,
}

async fn setup() -> Setup {
    setup_with(None, |_| {}).await
}

/// `limit` overrides how many tasks run at once; `prepare` runs on the data dir before the
/// volumes are discovered.
async fn setup_with(limit: Option<usize>, prepare: impl FnOnce(&Path)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    for name in ["media", "documents"] {
        fs::create_dir_all(data.join(name)).unwrap();
    }
    prepare(&data);
    let (mut state, fake) = common::state_with_volumes(dir.path(), &["media", "documents"]).await;
    if let Some(limit) = limit {
        state.tasks = Arc::new(Tasks::with_limit(limit));
    }
    let app = router(state).await.unwrap();
    let client = Client::signed_in(app.clone(), &fake).await;
    Setup {
        client,
        app,
        fake,
        data,
        _dir: dir,
    }
}

async fn post_task(client: &mut Client, body: serde_json::Value) -> Response {
    let req = Request::post("/api/tasks")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    client.send(req).await
}

async fn delete_task(client: &mut Client, id: &str) -> Response {
    let req = Request::delete(format!("/api/tasks/{id}"))
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    client.send(req).await
}

async fn list(client: &mut Client) -> Vec<TaskInfo> {
    let res = client.get("/api/tasks").await;
    assert_eq!(res.status(), StatusCode::OK);
    json(res).await
}

async fn error_of(res: Response, status: StatusCode, code: &str) -> ErrorBody {
    assert_eq!(res.status(), status);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, code);
    body
}

fn delete_spec(volume: &str, path: &str) -> serde_json::Value {
    json!({"op": "delete", "source": {"volume": volume, "path": path}})
}

async fn started(client: &mut Client, volume: &str, path: &str) -> TaskInfo {
    let res = post_task(client, delete_spec(volume, path)).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    assert_eq!(info.state, TaskState::Queued);
    assert_eq!(info.id.len(), 32);
    assert!(
        info.id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    );
    info
}

/// Polls the task list until `id` is finished.
async fn finished(client: &mut Client, id: &str) -> TaskInfo {
    for _ in 0..200 {
        let task = list(client)
            .await
            .into_iter()
            .find(|task| task.id == id)
            .expect("task listed");
        if !matches!(task.state, TaskState::Queued | TaskState::Running) {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("task {id} did not finish");
}

#[tokio::test]
async fn api_responses_are_never_cached() {
    let mut s = setup().await;
    let res = s.client.get("/api/tasks").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
    let res = s.client.get("/api/nothing-here").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");

    let dir = tempfile::tempdir().unwrap();
    let mut anonymous = Client::new(common::app_with_web_dir(dir.path()).await);
    let res = anonymous.get("/api/tasks").await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn tasks_require_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path()).await;
    let mut client = Client::new(app);
    error_of(
        client.get("/api/tasks").await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
    error_of(
        post_task(&mut client, delete_spec("media", "a.txt")).await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
    error_of(
        delete_task(&mut client, "0123456789abcdef0123456789abcdef").await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
    )
    .await;
}

#[tokio::test]
async fn delete_task_runs_to_done() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("Films/2026")).unwrap();
    fs::write(media.join("Films/2026/a.mkv"), b"a").unwrap();

    let info = started(&mut s.client, "media", "Films").await;
    assert_eq!(
        info.spec,
        TaskSpec::Delete {
            source: Location {
                volume: "media".to_owned(),
                path: "Films".to_owned(),
            }
        }
    );

    let done = finished(&mut s.client, &info.id).await;
    assert_eq!(done.state, TaskState::Done);
    assert_eq!((done.files_done, done.files_total), (1, Some(1)));
    assert_eq!((done.bytes_done, done.bytes_total), (0, None));
    assert_eq!(done.error, None);
    assert!(!media.join("Films").exists());
}

#[tokio::test]
async fn delete_checks_run_before_the_task_exists() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::write(media.join("a.txt"), b"a").unwrap();
    fs::create_dir(media.join(".explorer-uploads")).unwrap();
    std::os::unix::fs::symlink(".", media.join("rootlink")).unwrap();

    for (body, status, code) in [
        (
            delete_spec("media", "nope"),
            StatusCode::NOT_FOUND,
            "path_not_found",
        ),
        (
            delete_spec("media", ""),
            StatusCode::BAD_REQUEST,
            "invalid_path",
        ),
        (
            delete_spec("media", "../x"),
            StatusCode::BAD_REQUEST,
            "invalid_path",
        ),
        (
            delete_spec("media", ".explorer-uploads"),
            StatusCode::BAD_REQUEST,
            "invalid_path",
        ),
        (
            delete_spec("media", "rootlink/.explorer-uploads"),
            StatusCode::NOT_FOUND,
            "path_not_found",
        ),
        (
            delete_spec("nope", "a.txt"),
            StatusCode::NOT_FOUND,
            "volume_not_found",
        ),
        (
            json!({"op": "delete"}),
            StatusCode::BAD_REQUEST,
            "invalid_path",
        ),
        (
            json!({
                "op": "move",
                "source": {"volume": "media", "path": ""},
                "dest_dir": {"volume": "documents", "path": ""},
            }),
            StatusCode::BAD_REQUEST,
            "invalid_path",
        ),
    ] {
        let res = post_task(&mut s.client, body.clone()).await;
        let error = error_of(res, status, code).await;
        assert!(error.request_id.is_some(), "{body}");
    }
    assert!(list(&mut s.client).await.is_empty());
    assert!(s.data.join("media/a.txt").is_file());
}

#[tokio::test]
async fn failed_task_keeps_the_error_and_request_id() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup().await;
    let locked = s.data.join("media/locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    fs::write(locked.join("inner/a.txt"), b"a").unwrap();
    fs::set_permissions(locked.join("inner"), fs::Permissions::from_mode(0o555)).unwrap();

    let res = post_task(&mut s.client, delete_spec("media", "locked")).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let request_id = res.headers()["x-request-id"].to_str().unwrap().to_owned();
    let info: TaskInfo = json(res).await;
    let failed = finished(&mut s.client, &info.id).await;
    fs::set_permissions(locked.join("inner"), fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(failed.state, TaskState::Failed);
    let error = failed.error.unwrap();
    assert_eq!(error.code, "permission_denied");
    assert_eq!(error.request_id, Some(request_id));
    assert_eq!(failed.files_done, 0);
}

#[tokio::test]
async fn tasks_are_per_user() {
    let mut s = setup_with(Some(0), |_| {}).await;
    fs::write(s.data.join("media/a.txt"), b"a").unwrap();
    let info = started(&mut s.client, "media", "a.txt").await;

    s.fake.claims(json!({ "sub": "other-user" }));
    let mut other = Client::signed_in(s.app.clone(), &s.fake).await;
    assert!(list(&mut other).await.is_empty());
    error_of(
        delete_task(&mut other, &info.id).await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;

    let mine = list(&mut s.client).await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].state, TaskState::Queued);
}

#[tokio::test]
async fn delete_on_read_only_volume_is_403() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(None, |data| {
        fs::write(data.join("documents/a.txt"), b"a").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    let res = post_task(&mut s.client, delete_spec("documents", "a.txt")).await;
    error_of(res, StatusCode::FORBIDDEN, "read_only_volume").await;
    assert!(s.data.join("documents/a.txt").is_file());
    assert!(list(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn cancel_queued_task() {
    // No task can start, so every one stays queued until cancelled.
    let mut s = setup_with(Some(0), |_| {}).await;
    let media = s.data.join("media");
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(media.join(name), b"x").unwrap();
    }
    let first = started(&mut s.client, "media", "a.txt").await;
    started(&mut s.client, "media", "b.txt").await;
    let third = started(&mut s.client, "media", "c.txt").await;

    let res = delete_task(&mut s.client, &third.id).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let tasks = list(&mut s.client).await;
    let states: Vec<TaskState> = tasks.iter().map(|task| task.state).collect();
    assert_eq!(
        states,
        [TaskState::Queued, TaskState::Queued, TaskState::Cancelled]
    );
    assert_eq!(tasks[0].id, first.id);
    assert!(media.join("c.txt").is_file());
}

#[tokio::test]
async fn dismiss_finished_task() {
    let mut s = setup().await;
    fs::write(s.data.join("media/a.txt"), b"a").unwrap();
    let info = started(&mut s.client, "media", "a.txt").await;
    finished(&mut s.client, &info.id).await;

    let res = delete_task(&mut s.client, &info.id).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(list(&mut s.client).await.is_empty());

    error_of(
        delete_task(&mut s.client, &info.id).await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;
    error_of(
        delete_task(&mut s.client, "not-an-id").await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;
}

fn copy_spec(source: (&str, &str), dest_dir: (&str, &str)) -> serde_json::Value {
    json!({
        "op": "copy",
        "source": {"volume": source.0, "path": source.1},
        "dest_dir": {"volume": dest_dir.0, "path": dest_dir.1},
    })
}

#[tokio::test]
async fn copy_task_across_volumes_reports_progress() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("F/Sub")).unwrap();
    fs::write(media.join("F/a.txt"), b"abc").unwrap();
    fs::write(media.join("F/Sub/b.txt"), b"defg").unwrap();

    let res = post_task(&mut s.client, copy_spec(("media", "F"), ("documents", ""))).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    let done = finished(&mut s.client, &info.id).await;

    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!((done.files_done, done.files_total), (2, Some(2)));
    assert_eq!((done.bytes_done, done.bytes_total), (7, Some(7)));
    assert_eq!(done.skipped, 0);
    assert_eq!(done.final_path.as_deref(), Some("F"));
    let copied = s.data.join("documents/F");
    assert_eq!(fs::read(copied.join("a.txt")).unwrap(), b"abc");
    assert_eq!(fs::read(copied.join("Sub/b.txt")).unwrap(), b"defg");
    assert!(media.join("F/Sub/b.txt").is_file());
}

#[tokio::test]
async fn copy_conflict_is_409_with_details() {
    let mut s = setup().await;
    let (media, documents) = (s.data.join("media"), s.data.join("documents"));
    fs::write(media.join("a.txt"), b"new").unwrap();
    fs::create_dir(media.join("F")).unwrap();
    fs::write(documents.join("a.txt"), b"old").unwrap();
    fs::create_dir(documents.join("F")).unwrap();

    for (path, kind) in [("a.txt", "file"), ("F", "dir")] {
        let res = post_task(&mut s.client, copy_spec(("media", path), ("documents", ""))).await;
        let error = error_of(res, StatusCode::CONFLICT, "already_exists").await;
        assert_eq!(
            error.details,
            Some(json!({"volume": "documents", "path": path, "kind": kind}))
        );
    }

    let mut replace = copy_spec(("media", "F"), ("documents", ""));
    replace["conflict"] = json!("replace");
    let res = post_task(&mut s.client, replace).await;
    error_of(res, StatusCode::BAD_REQUEST, "is_a_directory").await;

    let res = post_task(&mut s.client, copy_spec(("media", "F"), ("media", "F"))).await;
    let error = error_of(res, StatusCode::BAD_REQUEST, "invalid_destination").await;
    assert_eq!(error.details, Some(json!({"volume": "media", "path": "F"})));
    assert!(list(&mut s.client).await.is_empty());
    assert_eq!(fs::read(documents.join("a.txt")).unwrap(), b"old");
}

#[tokio::test]
async fn copy_from_read_only_volume_is_allowed() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(None, |data| {
        fs::write(data.join("documents/a.txt"), b"a").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;

    let res = post_task(
        &mut s.client,
        copy_spec(("documents", "a.txt"), ("media", "")),
    )
    .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    let done = finished(&mut s.client, &info.id).await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!(done.final_path.as_deref(), Some("a.txt"));
    assert_eq!(fs::read(s.data.join("media/a.txt")).unwrap(), b"a");
}

#[tokio::test]
async fn copy_to_read_only_volume_is_403() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(None, |data| {
        fs::write(data.join("media/a.txt"), b"a").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    let res = post_task(
        &mut s.client,
        copy_spec(("media", "a.txt"), ("documents", "")),
    )
    .await;
    error_of(res, StatusCode::FORBIDDEN, "read_only_volume").await;
    assert!(!s.data.join("documents/a.txt").exists());
    assert!(list(&mut s.client).await.is_empty());
}

fn move_spec(source: (&str, &str), dest_dir: (&str, &str)) -> serde_json::Value {
    let mut spec = copy_spec(source, dest_dir);
    spec["op"] = json!("move");
    spec
}

#[tokio::test]
async fn move_task_same_volume() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("Sub")).unwrap();
    fs::write(media.join("a.txt"), b"abc").unwrap();
    fs::write(media.join("Sub/b.txt"), b"b").unwrap();

    let res = post_task(
        &mut s.client,
        move_spec(("media", "a.txt"), ("media", "Sub")),
    )
    .await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    let done = finished(&mut s.client, &info.id).await;

    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!((done.files_done, done.files_total), (1, Some(1)));
    assert_eq!(done.final_path.as_deref(), Some("Sub/a.txt"));
    assert_eq!(fs::read(media.join("Sub/a.txt")).unwrap(), b"abc");
    assert!(!media.join("a.txt").exists());

    let res = post_task(
        &mut s.client,
        move_spec(("media", "Sub/b.txt"), ("media", "Sub")),
    )
    .await;
    let error = error_of(res, StatusCode::BAD_REQUEST, "invalid_destination").await;
    assert_eq!(
        error.details,
        Some(json!({"volume": "media", "path": "Sub/b.txt"}))
    );
    let res = post_task(&mut s.client, move_spec(("media", "Sub"), ("media", "Sub"))).await;
    error_of(res, StatusCode::BAD_REQUEST, "invalid_destination").await;
    fs::write(media.join("b.txt"), b"other").unwrap();
    let res = post_task(
        &mut s.client,
        move_spec(("media", "Sub/b.txt"), ("media", "")),
    )
    .await;
    let error = error_of(res, StatusCode::CONFLICT, "already_exists").await;
    assert_eq!(
        error.details,
        Some(json!({"volume": "media", "path": "b.txt", "kind": "file"}))
    );
    assert_eq!(list(&mut s.client).await.len(), 1);
}

#[tokio::test]
async fn move_task_across_volumes() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("F/Sub")).unwrap();
    fs::write(media.join("F/a.txt"), b"abc").unwrap();
    fs::write(media.join("F/Sub/b.txt"), b"defg").unwrap();

    let res = post_task(&mut s.client, move_spec(("media", "F"), ("documents", ""))).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    let done = finished(&mut s.client, &info.id).await;

    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!((done.files_done, done.files_total), (2, Some(2)));
    assert_eq!((done.bytes_done, done.bytes_total), (7, Some(7)));
    assert_eq!(done.final_path.as_deref(), Some("F"));
    let moved = s.data.join("documents/F");
    assert_eq!(fs::read(moved.join("a.txt")).unwrap(), b"abc");
    assert_eq!(fs::read(moved.join("Sub/b.txt")).unwrap(), b"defg");
    assert!(!media.join("F").exists());
}

#[tokio::test]
async fn move_from_read_only_volume_is_403() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(None, |data| {
        fs::write(data.join("documents/a.txt"), b"a").unwrap();
        fs::write(data.join("media/b.txt"), b"b").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();

    for spec in [
        move_spec(("documents", "a.txt"), ("media", "")),
        move_spec(("media", "b.txt"), ("documents", "")),
    ] {
        let res = post_task(&mut s.client, spec).await;
        error_of(res, StatusCode::FORBIDDEN, "read_only_volume").await;
    }
    assert!(s.data.join("documents/a.txt").is_file());
    assert!(s.data.join("media/b.txt").is_file());
    assert!(!s.data.join("media/a.txt").exists());
    assert!(list(&mut s.client).await.is_empty());
}

fn usage_spec(volume: &str, path: &str) -> serde_json::Value {
    json!({"op": "usage", "source": {"volume": volume, "path": path}})
}

async fn usage_task(client: &mut Client, volume: &str, path: &str) -> TaskInfo {
    let res = post_task(client, usage_spec(volume, path)).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let info: TaskInfo = json(res).await;
    assert_eq!(
        info.spec,
        TaskSpec::Usage {
            source: Location {
                volume: volume.to_owned(),
                path: path.to_owned(),
            }
        }
    );
    info
}

#[tokio::test]
async fn usage_task_runs_and_is_browsable() {
    let mut s = setup().await;
    let media = s.data.join("media");
    fs::create_dir_all(media.join("Films/2026")).unwrap();
    fs::write(media.join("Films/a.mkv"), vec![1u8; 20_000]).unwrap();
    fs::write(media.join("Films/2026/b.mkv"), vec![1u8; 5_000]).unwrap();

    let info = usage_task(&mut s.client, "media", "").await;
    let done = finished(&mut s.client, &info.id).await;
    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!((done.files_done, done.files_total), (2, None));
    assert_eq!(done.bytes_total, None);

    let res = s.client.get(&format!("/api/tasks/{}/usage", info.id)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let root: UsageView = json(res).await;
    assert_eq!(
        (root.volume.as_str(), root.root.as_str(), root.path.as_str()),
        ("media", "", "")
    );
    assert_eq!((root.files, root.dirs), (2, 2));
    assert!(root.scanned_at_ms > 0 && !root.incomplete && !root.collapsed);
    assert_eq!(root.items.len(), 1);
    assert_eq!(
        (root.items[0].name.as_str(), root.items[0].kind),
        ("Films", UsageKind::Dir)
    );
    assert_eq!(root.items[0].bytes + allocated(&media), root.bytes);
    assert_eq!(done.bytes_done, root.bytes);

    let res = s
        .client
        .get(&format!("/api/tasks/{}/usage?path=Films", info.id))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let films: UsageView = json(res).await;
    assert_eq!((films.root.as_str(), films.path.as_str()), ("", "Films"));
    let names: Vec<(&str, UsageKind)> = films
        .items
        .iter()
        .map(|item| (item.name.as_str(), item.kind))
        .collect();
    assert_eq!(
        names,
        [("a.mkv", UsageKind::File), ("2026", UsageKind::Dir)]
    );

    for path in ["Films/a.mkv", "nope"] {
        let res = s
            .client
            .get(&format!("/api/tasks/{}/usage?path={path}", info.id))
            .await;
        error_of(res, StatusCode::NOT_FOUND, "path_not_found").await;
    }
}

fn allocated(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

#[tokio::test]
async fn usage_of_a_file_is_400() {
    let mut s = setup().await;
    fs::write(s.data.join("media/a.txt"), b"a").unwrap();
    let res = post_task(&mut s.client, usage_spec("media", "a.txt")).await;
    error_of(res, StatusCode::BAD_REQUEST, "not_a_directory").await;
    let res = post_task(&mut s.client, usage_spec("media", "nope")).await;
    error_of(res, StatusCode::NOT_FOUND, "path_not_found").await;
    assert!(list(&mut s.client).await.is_empty());
}

#[tokio::test]
async fn usage_on_read_only_volume_is_allowed() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let mut s = setup_with(None, |data| {
        fs::write(data.join("documents/a.txt"), b"a").unwrap();
        fs::set_permissions(data.join("documents"), fs::Permissions::from_mode(0o555)).unwrap();
    })
    .await;
    let info = usage_task(&mut s.client, "documents", "").await;
    let done = finished(&mut s.client, &info.id).await;
    fs::set_permissions(s.data.join("documents"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(done.state, TaskState::Done, "{:?}", done.error);
    assert_eq!(done.files_done, 1);
}

#[tokio::test]
async fn usage_of_another_users_task_is_404() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/F")).unwrap();
    let info = usage_task(&mut s.client, "media", "F").await;
    finished(&mut s.client, &info.id).await;
    let uri = format!("/api/tasks/{}/usage", info.id);
    assert_eq!(s.client.get(&uri).await.status(), StatusCode::OK);

    s.fake.claims(json!({ "sub": "other-user" }));
    let mut other = Client::signed_in(s.app.clone(), &s.fake).await;
    error_of(
        other.get(&uri).await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;
}

#[tokio::test]
async fn usage_before_done_is_404() {
    let mut s = setup_with(Some(0), |_| {}).await;
    let info = usage_task(&mut s.client, "media", "").await;
    error_of(
        s.client.get(&format!("/api/tasks/{}/usage", info.id)).await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;

    let mut s = setup().await;
    fs::write(s.data.join("media/a.txt"), b"a").unwrap();
    let deleted = started(&mut s.client, "media", "a.txt").await;
    finished(&mut s.client, &deleted.id).await;
    for id in [deleted.id.as_str(), "0123456789abcdef0123456789abcdef"] {
        error_of(
            s.client.get(&format!("/api/tasks/{id}/usage")).await,
            StatusCode::NOT_FOUND,
            "task_not_found",
        )
        .await;
    }
}

#[tokio::test]
async fn rescanning_a_folder_drops_the_older_tree() {
    let mut s = setup().await;
    fs::create_dir(s.data.join("media/F")).unwrap();
    let first = usage_task(&mut s.client, "media", "F").await;
    finished(&mut s.client, &first.id).await;
    let second = usage_task(&mut s.client, "media", "F").await;
    finished(&mut s.client, &second.id).await;

    error_of(
        s.client
            .get(&format!("/api/tasks/{}/usage", first.id))
            .await,
        StatusCode::NOT_FOUND,
        "task_not_found",
    )
    .await;
    let res = s
        .client
        .get(&format!("/api/tasks/{}/usage", second.id))
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(list(&mut s.client).await.len(), 2);
}
