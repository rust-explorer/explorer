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

#[tokio::test]
async fn open_fails_clearly_on_read_only_database_file() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("explorer.db");
    std::fs::write(&file, b"").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();

    let err = db::open(dir.path()).await.unwrap_err();

    assert!(matches!(err, DbError::NotWritable { .. }), "{err:?}");
    let message = err.to_string();
    assert!(message.contains(file.to_str().unwrap()), "{message}");
    assert!(message.contains("owner uid"), "{message}");
    assert!(message.contains("check the volume mount"), "{message}");
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn database_files_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(dir.path()).await.unwrap();
    sqlx::query("CREATE TABLE probe (a)")
        .execute(&pool)
        .await
        .unwrap();

    for name in ["explorer.db", "explorer.db-wal", "explorer.db-shm"] {
        assert_eq!(mode(&dir.path().join(name)), 0o600, "{name}");
    }
}

#[tokio::test]
async fn existing_database_is_made_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    drop(db::open(dir.path()).await.unwrap());
    let file = dir.path().join("explorer.db");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    db::open(dir.path()).await.unwrap();

    assert_eq!(mode(&file), 0o600);
}
