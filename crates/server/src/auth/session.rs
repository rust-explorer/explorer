use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tower_sessions::cookie::SameSite;
use tower_sessions::{ExpiredDeletion as _, Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;

use crate::http::error::ApiError;

pub const COOKIE_NAME: &str = "explorer_session";
const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
/// Browsers cap cookie lifetimes at 400 days, so a longer TTL buys nothing.
const MAX_TTL: Duration = Duration::from_secs(400 * 24 * 3600);
/// However active, a session ends this long after sign-in (or after the TTL, if longer), so that
/// a change of groups at the provider is eventually picked up.
const MAX_LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);
/// Activity pushes the expiry back at most this often, to spare a database write per request.
const MAX_TOUCH_INTERVAL: Duration = Duration::from_secs(3600);
const STAMPS_KEY: &str = "lifetime";

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

/// When a signed-in session was started and last pushed back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamps {
    signed_in_at: OffsetDateTime,
    touched_at: OffsetDateTime,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Keep,
    Touch,
    End,
}

/// The sliding expiry of signed-in sessions, bounded by a maximum lifetime.
#[derive(Clone, Copy, Debug)]
pub struct Lifetime {
    ttl: time::Duration,
    max: time::Duration,
    touch_every: time::Duration,
}

impl Lifetime {
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        let ttl = inactivity(ttl);
        Self {
            ttl,
            max: inactivity(MAX_LIFETIME).max(ttl),
            touch_every: (ttl / 2_u32).min(inactivity(MAX_TOUCH_INTERVAL)),
        }
    }

    fn verdict(&self, stamps: Stamps, now: OffsetDateTime) -> Verdict {
        if now - stamps.signed_in_at >= self.max {
            Verdict::End
        } else if now - stamps.touched_at >= self.touch_every {
            Verdict::Touch
        } else {
            Verdict::Keep
        }
    }

    fn remaining(&self, stamps: Stamps, now: OffsetDateTime) -> time::Duration {
        self.ttl
            .min(stamps.signed_in_at + self.max - now)
            .max(time::Duration::ZERO)
    }

    /// Ends a session past its maximum lifetime, and marks an active one for saving now and then.
    async fn check(&self, session: &Session) -> Result<(), tower_sessions::session::Error> {
        let Some(stamps) = session.get::<Stamps>(STAMPS_KEY).await? else {
            return Ok(());
        };
        let now = OffsetDateTime::now_utc();
        match self.verdict(stamps, now) {
            Verdict::Keep => {}
            Verdict::Touch => {
                let touched = Stamps {
                    touched_at: now,
                    ..stamps
                };
                session.insert(STAMPS_KEY, touched).await?;
            }
            Verdict::End => session.flush().await?,
        }
        Ok(())
    }

    /// A signed-in session about to be saved lasts the TTL from now, within its maximum lifetime.
    async fn set_expiry(&self, session: &Session) -> Result<(), tower_sessions::session::Error> {
        if let Some(stamps) = session.get::<Stamps>(STAMPS_KEY).await? {
            let remaining = self.remaining(stamps, OffsetDateTime::now_utc());
            session.set_expiry(Some(Expiry::OnInactivity(remaining)));
        }
        Ok(())
    }
}

/// Starts the lifetime of a session that now carries an identity.
pub async fn start(session: &Session) -> Result<(), tower_sessions::session::Error> {
    let now = OffsetDateTime::now_utc();
    let stamps = Stamps {
        signed_in_at: now,
        touched_at: now,
    };
    session.insert(STAMPS_KEY, stamps).await
}

/// Applies [`Lifetime`] around each request; runs inside the session layer.
pub async fn keep_alive(
    State(lifetime): State<Lifetime>,
    session: Session,
    req: Request,
    next: Next,
) -> Response {
    if session.id().is_some()
        && let Err(err) = lifetime.check(&session).await
    {
        return ApiError::internal(err).into_response();
    }
    let res = next.run(req).await;
    if session.is_modified()
        && let Err(err) = lifetime.set_expiry(&session).await
    {
        return ApiError::internal(err).into_response();
    }
    res
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

    fn stamps(signed_in_days_ago: i64, touched_minutes_ago: i64) -> (Stamps, OffsetDateTime) {
        let now = OffsetDateTime::now_utc();
        let stamps = Stamps {
            signed_in_at: now - time::Duration::days(signed_in_days_ago),
            touched_at: now - time::Duration::minutes(touched_minutes_ago),
        };
        (stamps, now)
    }

    #[test]
    fn activity_touches_at_most_hourly() {
        let lifetime = Lifetime::new(Duration::from_secs(7 * 24 * 3600));
        let (fresh, now) = stamps(1, 59);
        assert_eq!(lifetime.verdict(fresh, now), Verdict::Keep);
        let (due, now) = stamps(1, 60);
        assert_eq!(lifetime.verdict(due, now), Verdict::Touch);
    }

    #[test]
    fn short_ttls_touch_at_half_the_ttl() {
        let lifetime = Lifetime::new(Duration::from_secs(60));
        assert_eq!(lifetime.touch_every, time::Duration::seconds(30));
    }

    #[test]
    fn sessions_end_after_the_maximum_lifetime_however_active() {
        let lifetime = Lifetime::new(Duration::from_secs(7 * 24 * 3600));
        let (active, now) = stamps(30, 0);
        assert_eq!(lifetime.verdict(active, now), Verdict::End);
        let (almost, now) = stamps(29, 0);
        assert_eq!(lifetime.verdict(almost, now), Verdict::Keep);
    }

    #[test]
    fn expiry_never_passes_the_maximum_lifetime() {
        let lifetime = Lifetime::new(Duration::from_secs(7 * 24 * 3600));
        let (early, now) = stamps(1, 0);
        assert_eq!(lifetime.remaining(early, now), time::Duration::days(7));
        let (late, now) = stamps(28, 0);
        assert_eq!(lifetime.remaining(late, now), time::Duration::days(2));
        let (over, now) = stamps(31, 0);
        assert_eq!(lifetime.remaining(over, now), time::Duration::ZERO);
    }

    #[test]
    fn maximum_lifetime_is_at_least_the_ttl() {
        let lifetime = Lifetime::new(Duration::from_secs(90 * 24 * 3600));
        assert_eq!(lifetime.max, time::Duration::days(90));
        let lifetime = Lifetime::new(Duration::from_secs(24 * 3600));
        assert_eq!(lifetime.max, time::Duration::days(30));
    }
}
