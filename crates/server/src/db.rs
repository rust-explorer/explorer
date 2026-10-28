use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

const DB_FILE: &str = "explorer.db";
const WAL_FILE: &str = "explorer.db-wal";
const SHM_FILE: &str = "explorer.db-shm";
const WRITE_TEST_FILE: &str = ".write-test";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// The database holds live session ids: only the server's user may read it.
const FILE_MODE: u32 = 0o600;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(
        "{path} is not writable (owner {owner}; running as uid {uid}, \
         gid {gid}): {source}; check the volume mount and the container `user:`",
        path = path.display(),
        uid = rustix::process::geteuid().as_raw(),
        gid = rustix::process::getegid().as_raw(),
    )]
    NotWritable {
        path: PathBuf,
        owner: String,
        source: io::Error,
    },
    #[error("cannot open database: {0}")]
    Open(#[from] sqlx::Error),
    #[error("cannot run database migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Opens `{config_dir}/explorer.db` in WAL mode, creating it if needed, and applies migrations.
pub async fn open(config_dir: &Path) -> Result<SqlitePool, DbError> {
    check_writable(config_dir)?;
    restrict_files(config_dir).map_err(|source| not_writable(config_dir, source))?;
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

/// Fails early, with an actionable message, instead of surfacing `SQLite`'s "readonly database".
fn check_writable(dir: &Path) -> Result<(), DbError> {
    let probe = dir.join(WRITE_TEST_FILE);
    std::fs::File::create(&probe)
        .and_then(|_| std::fs::remove_file(&probe))
        .map_err(|source| not_writable(dir, source))?;
    for name in [DB_FILE, WAL_FILE, SHM_FILE] {
        let path = dir.join(name);
        match OpenOptions::new().write(true).open(&path) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                return Err(not_writable(&path, source));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Creates the database file owner-only (`SQLite` gives `-wal` and `-shm` the same mode) and
/// tightens files left by an earlier version.
fn restrict_files(dir: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .open(dir.join(DB_FILE))?;
    for name in [DB_FILE, WAL_FILE, SHM_FILE] {
        let path = dir.join(name);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.permissions().mode() & 0o777 != FILE_MODE => {
                if let Err(err) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(FILE_MODE))
                {
                    tracing::warn!(path = %path.display(), error = %err, "cannot make the database owner-only");
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn not_writable(path: &Path, source: io::Error) -> DbError {
    let owner = std::fs::metadata(path).map_or_else(
        |_| "unknown".to_owned(),
        |m| format!("uid {}, gid {}", m.uid(), m.gid()),
    );
    DbError::NotWritable {
        path: path.to_path_buf(),
        owner,
        source,
    }
}
