use std::time::Duration;

use sqlx::SqlitePool;
use tower_sessions::cookie::SameSite;
use tower_sessions::{ExpiredDeletion as _, Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;

pub const COOKIE_NAME: &str = "explorer_session";
const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
/// Browsers cap cookie lifetimes at 400 days, so a longer TTL buys nothing.
const MAX_TTL: Duration = Duration::from_secs(400 * 24 * 3600);

/// Builds the session layer over the `SQLite` store, creating its table if needed.
pub async fn layer(
    db: SqlitePool,
    ttl: Duration,
) -> Result<SessionManagerLayer<SqliteStore>, sqlx::Error> {
    let store = SqliteStore::new(db);
    store.migrate().await?;
    Ok(SessionManagerLayer::new(store)
        .with_name(COOKIE_NAME)
        .with_http_only(true)
        .with_secure(true)
        .with_same_site(SameSite::Lax)
        .with_path("/")
        .with_expiry(Expiry::OnInactivity(inactivity(ttl))))
}

fn inactivity(ttl: Duration) -> time::Duration {
    time::Duration::try_from(ttl.min(MAX_TTL)).expect("MAX_TTL fits in time::Duration")
}

/// Deletes expired sessions every hour; a failed sweep is logged and retried at the next tick.
#[must_use = "dropping the handle detaches the task"]
pub fn spawn_expired_cleanup(db: SqlitePool) -> tokio::task::JoinHandle<()> {
    let store = SqliteStore::new(db);
    tokio::spawn(async move {
        loop {
            if let Err(err) = store
                .clone()
                .continuously_delete_expired(CLEANUP_INTERVAL)
                .await
            {
                tracing::warn!(error = %err, "expired session cleanup failed");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_is_kept_when_reasonable() {
        let ttl = Duration::from_secs(7 * 24 * 3600);
        assert_eq!(inactivity(ttl), time::Duration::days(7));
    }

    #[test]
    fn huge_ttl_is_capped() {
        assert_eq!(inactivity(Duration::MAX), time::Duration::days(400));
    }
}
