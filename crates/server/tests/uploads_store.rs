use std::sync::Arc;
use std::time::{Duration, SystemTime};

use explorer_server::db;
use explorer_server::storage::{RelPath, StagedId, Volumes};
use explorer_server::uploads::{SweepReport, UPLOAD_TTL, UploadRecord, Uploads, sweep};
use shared::api::conflict::ConflictPolicy;

const HOUR_MS: i64 = 3_600_000;

struct Fixture {
    _dir: tempfile::TempDir,
    uploads: Uploads,
    volumes: Arc<Volumes>,
}

async fn fixture(volumes: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    std::fs::create_dir(&config).unwrap();
    for name in volumes {
        std::fs::create_dir_all(data.join(name)).unwrap();
    }
    let pool = db::open(&config).await.unwrap();
    Fixture {
        uploads: Uploads::new(pool),
        volumes: Arc::new(Volumes::discover(&data).unwrap()),
        _dir: dir,
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn record(user: &str, volume: &str, updated_at: i64) -> UploadRecord {
    UploadRecord {
        id: StagedId::random(),
        user_sub: user.to_owned(),
        volume: volume.to_owned(),
        path: RelPath::parse("Film.mkv").unwrap(),
        length: 10,
        modified_ms: Some(1_700_000_000_000),
        conflict: ConflictPolicy::KeepBoth,
        final_path: None,
        created_at: updated_at,
        updated_at,
    }
}

fn stage(fx: &Fixture, volume: &str, id: &StagedId, age: Duration) {
    let staging = fx.volumes.get(volume).unwrap().staging().unwrap();
    let file = staging.create(id).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

fn staged(fx: &Fixture, volume: &str, id: &StagedId) -> bool {
    let staging = fx.volumes.get(volume).unwrap().existing_staging().unwrap();
    staging.is_some_and(|s| s.len(id).unwrap().is_some())
}

#[tokio::test]
async fn get_is_scoped_to_the_user() {
    let fx = fixture(&["media"]).await;
    let rec = record("alice", "media", now_ms());
    fx.uploads.insert(&rec).await.unwrap();

    assert_eq!(
        fx.uploads.get(&rec.id, "alice").await.unwrap(),
        Some(rec.clone())
    );
    assert_eq!(fx.uploads.get(&rec.id, "bob").await.unwrap(), None);
}

#[tokio::test]
async fn list_pending_excludes_done() {
    let fx = fixture(&["media"]).await;
    let now = now_ms();
    let newer = record("alice", "media", now);
    let older = record("alice", "media", now - HOUR_MS);
    let done = record("alice", "media", now - 2 * HOUR_MS);
    let other = record("bob", "media", now);
    for rec in [&newer, &older, &done, &other] {
        fx.uploads.insert(rec).await.unwrap();
    }
    fx.uploads
        .mark_done(&done.id, &RelPath::parse("Film (1).mkv").unwrap(), now)
        .await
        .unwrap();

    let pending = fx.uploads.list_pending("alice").await.unwrap();
    assert_eq!(pending, vec![older, newer]);
    let got = fx.uploads.get(&done.id, "alice").await.unwrap().unwrap();
    assert_eq!(
        got.final_path,
        Some(RelPath::parse("Film (1).mkv").unwrap())
    );
}

#[tokio::test]
async fn touch_and_delete() {
    let fx = fixture(&["media"]).await;
    let rec = record("alice", "media", 1);
    fx.uploads.insert(&rec).await.unwrap();
    fx.uploads.touch(&rec.id, 42).await.unwrap();
    assert_eq!(
        fx.uploads
            .get(&rec.id, "alice")
            .await
            .unwrap()
            .unwrap()
            .updated_at,
        42
    );
    fx.uploads.delete(&rec.id).await.unwrap();
    assert_eq!(fx.uploads.get(&rec.id, "alice").await.unwrap(), None);
}

#[tokio::test]
async fn sweep_removes_expired_uploads_and_their_files() {
    let fx = fixture(&["media"]).await;
    let now = now_ms();
    let old = record("alice", "media", now - 25 * HOUR_MS);
    let fresh = record("alice", "media", now - HOUR_MS);
    for rec in [&old, &fresh] {
        fx.uploads.insert(rec).await.unwrap();
        stage(&fx, "media", &rec.id, Duration::from_secs(60));
    }

    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;

    assert_eq!(report.uploads, 1);
    assert_eq!(report.orphans, 0);
    assert_eq!(fx.uploads.get(&old.id, "alice").await.unwrap(), None);
    assert!(!staged(&fx, "media", &old.id));
    assert!(fx.uploads.get(&fresh.id, "alice").await.unwrap().is_some());
    assert!(staged(&fx, "media", &fresh.id));
}

#[tokio::test]
async fn sweep_skips_locked_uploads() {
    let fx = fixture(&["media"]).await;
    let old = record("alice", "media", now_ms() - 25 * HOUR_MS);
    fx.uploads.insert(&old).await.unwrap();
    stage(&fx, "media", &old.id, UPLOAD_TTL + Duration::from_secs(60));

    let lock = fx.uploads.try_lock(&old.id).unwrap();
    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;
    assert_eq!(report, SweepReport::default());
    assert!(fx.uploads.get(&old.id, "alice").await.unwrap().is_some());
    assert!(staged(&fx, "media", &old.id));

    drop(lock);
    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;
    assert_eq!(report.uploads, 1);
    assert!(!staged(&fx, "media", &old.id));
}

#[tokio::test]
async fn sweep_removes_records_of_missing_volumes() {
    let fx = fixture(&["media"]).await;
    let old = record("alice", "gone", now_ms() - 25 * HOUR_MS);
    fx.uploads.insert(&old).await.unwrap();

    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;

    assert_eq!(report.uploads, 1);
    assert_eq!(fx.uploads.get(&old.id, "alice").await.unwrap(), None);
}

#[tokio::test]
async fn sweep_removes_old_orphans_only() {
    let fx = fixture(&["media"]).await;
    let old = StagedId::random();
    let fresh = StagedId::random();
    stage(&fx, "media", &old, UPLOAD_TTL + Duration::from_secs(3600));
    stage(&fx, "media", &fresh, Duration::from_secs(3600));

    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;

    assert_eq!(report.orphans, 1);
    assert_eq!(report.uploads, 0);
    assert!(!staged(&fx, "media", &old));
    assert!(staged(&fx, "media", &fresh));
}

#[tokio::test]
async fn sweep_keeps_old_files_that_have_a_record() {
    let fx = fixture(&["media"]).await;
    let rec = record("alice", "media", now_ms());
    fx.uploads.insert(&rec).await.unwrap();
    stage(
        &fx,
        "media",
        &rec.id,
        UPLOAD_TTL + Duration::from_secs(3600),
    );

    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;

    assert_eq!(report.orphans, 0);
    assert!(staged(&fx, "media", &rec.id));
}

#[tokio::test]
async fn sweep_never_creates_staging_dirs() {
    let fx = fixture(&["media", "documents"]).await;
    fx.uploads
        .insert(&record("alice", "media", now_ms() - 25 * HOUR_MS))
        .await
        .unwrap();

    let report = sweep(&fx.uploads, &fx.volumes, SystemTime::now()).await;

    assert_eq!(report.uploads, 1);
    for name in ["media", "documents"] {
        assert!(
            fx.volumes
                .get(name)
                .unwrap()
                .existing_staging()
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn lock_is_exclusive_until_dropped() {
    let fx = fixture(&["media"]).await;
    let id = StagedId::random();
    let other = StagedId::random();

    let lock = fx.uploads.try_lock(&id).unwrap();
    assert!(fx.uploads.try_lock(&id).is_none());
    assert!(fx.uploads.try_lock(&other).is_some());
    drop(lock);
    assert!(fx.uploads.try_lock(&id).is_some());
}
