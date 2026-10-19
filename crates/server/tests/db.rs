use std::os::unix::fs::PermissionsExt;

use explorer_server::db::{self, DbError};

#[tokio::test]
async fn open_creates_database_and_runs_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(dir.path()).await.unwrap();

    assert!(dir.path().join("explorer.db").exists());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_info")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn open_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    db::open(dir.path()).await.unwrap();
    let pool = db::open(dir.path()).await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_info")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn open_fails_clearly_on_read_only_dir() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

    let err = db::open(dir.path()).await.unwrap_err();

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(err, DbError::NotWritable { .. }), "{err:?}");
    let message = err.to_string();
    assert!(message.contains(dir.path().to_str().unwrap()), "{message}");
    assert!(message.contains("check the volume mount"), "{message}");
}
