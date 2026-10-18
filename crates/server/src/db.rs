use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

const DB_FILE: &str = "explorer.db";
const WRITE_TEST_FILE: &str = ".write-test";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(
        "config directory {dir} is not writable (uid {uid}, gid {gid}): {source}; \
         check the volume mount and the container `user:`",
        dir = dir.display(),
        uid = rustix::process::geteuid().as_raw(),
        gid = rustix::process::getegid().as_raw(),
    )]
    NotWritable { dir: PathBuf, source: io::Error },
    #[error("cannot open database: {0}")]
    Open(#[from] sqlx::Error),
    #[error("cannot run database migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Opens `{config_dir}/explorer.db` in WAL mode, creating it if needed, and applies migrations.
pub async fn open(config_dir: &Path) -> Result<SqlitePool, DbError> {
    check_writable(config_dir)?;
    let options = SqliteConnectOptions::new()
        .filename(config_dir.join(DB_FILE))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT);
    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

fn check_writable(dir: &Path) -> Result<(), DbError> {
    let probe = dir.join(WRITE_TEST_FILE);
    std::fs::File::create(&probe)
        .and_then(|_| std::fs::remove_file(&probe))
        .map_err(|source| DbError::NotWritable {
            dir: dir.to_path_buf(),
            source,
        })
}
