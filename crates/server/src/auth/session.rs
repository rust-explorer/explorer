use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tower_sessions::cookie::SameSite;
use tower_sessions::{
    ExpiredDeletion as _, Expiry, Session, SessionManagerLayer, SessionStore as _,
};
use tower_sessions_sqlx_store::SqliteStore;

use crate::http::error::ApiError;

pub const COOKIE_NAME: &str = "explorer_session";
/// Short, because anonymous sessions only hold sign-ins in progress.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(600);
/// How long an anonymous session, which holds only sign-ins in progress, lives: anyone can
/// create one, so it must not stay in the database for the full TTL.
const PENDING_TTL: Duration = Duration::from_secs(600);
/// Browsers cap cookie lifetimes at 400 days, so a longer TTL buys nothing.
const MAX_TTL: Duration = Duration::from_hours(400 * 24);
/// However active, a session ends this long after sign-in (or after the TTL, if longer), so that
/// a change of groups at the provider is eventually picked up.
const MAX_LIFETIME: Duration = Duration::from_hours(30 * 24);
/// Activity pushes the expiry back at most this often, to spare a database write per request.
const MAX_TOUCH_INTERVAL: Duration = Duration::from_secs(3600);
const STAMPS_KEY: &str = "lifetime";

/// Builds the session layer over the `SQLite` store, creating its table if needed. Sessions get
/// the short anonymous expiry unless [`keep_alive`] finds an identity in them.
pub async fn layer(db: SqlitePool) -> Result<SessionManagerLayer<SqliteStore>, sqlx::Error> {
    let store = SqliteStore::new(db);
    store.migrate().await?;
    Ok(SessionManagerLayer::new(store)
        .with_name(COOKIE_NAME)
        .with_http_only(true)
        .with_secure(true)
        .with_same_site(SameSite::Lax)
        .with_path("/")
        .with_expiry(Expiry::OnInactivity(inactivity(PENDING_TTL))))
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
    /// Returns whether it did the latter.
    async fn check(&self, session: &Session) -> Result<bool, tower_sessions::session::Error> {
        let Some(stamps) = session.get::<Stamps>(STAMPS_KEY).await? else {
            return Ok(false);
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
                return Ok(true);
            }
            Verdict::End => session.flush().await?,
        }
        Ok(false)
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

/// What [`keep_alive`] needs: the lifetime rules and the store, to check that a session it
/// touched still exists.
#[derive(Clone, Debug)]
pub struct KeepAlive {
    lifetime: Lifetime,
    store: SqliteStore,
}

impl KeepAlive {
    #[must_use]
    pub fn new(ttl: Duration, db: SqlitePool) -> Self {
        Self {
            lifetime: Lifetime::new(ttl),
            store: SqliteStore::new(db),
        }
    }
}

/// Applies [`Lifetime`] around each request; runs inside the session layer.
pub async fn keep_alive(
    State(keep): State<KeepAlive>,
    session: Session,
    req: Request,
    next: Next,
) -> Response {
    if !uses_session(req.uri().path()) {
        return next.run(req).await;
    }
    let touched = match session.id() {
        Some(_) => match keep.lifetime.check(&session).await {
            Ok(touched) => touched,
            Err(err) => return ApiError::internal(err).into_response(),
        },
        None => false,
    };
    let res = next.run(req).await;
    // A sign-out while this request ran removed the session: saving the touch would bring it
    // back, since the store saves with an upsert.
    if touched && let Some(id) = session.id() {
        match keep.store.load(&id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(err) = session.flush().await {
                    return ApiError::internal(err).into_response();
                }
                return res;
            }
            Err(err) => return ApiError::internal(err).into_response(),
        }
    }
    if session.is_modified()
        && let Err(err) = keep.lifetime.set_expiry(&session).await
    {
        return ApiError::internal(err).into_response();
    }
    res
}

/// Static files and `/healthz` never read the session, so they skip the database.
fn uses_session(path: &str) -> bool {
    ["/api", "/files", "/auth"].iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Deletes expired sessions every hour; a failed sweep is logged and retried at the next tick.
/// The task is watched: if it ends or panics, that is logged at error.
#[must_use = "dropping the handle detaches the task"]
pub fn spawn_expired_cleanup(db: SqlitePool) -> tokio::task::JoinHandle<()> {
    let store = SqliteStore::new(db);
    let cleanup = tokio::spawn(async move {
        loop {
            if let Err(err) = store
                .clone()
                .continuously_delete_expired(CLEANUP_INTERVAL)
                .await
            {
                tracing::warn!(error = %err, "expired session cleanup failed");
            }
        }
    });
    watch(cleanup)
}

/// Spawns a task that logs at error when `task` ends, which it should never do.
fn watch(task: tokio::task::JoinHandle<()>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match task.await {
            Ok(()) => tracing::error!("expired session cleanup stopped"),
            Err(err) => tracing::error!(error = %err, "expired session cleanup crashed"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_api_files_and_auth_use_the_session() {
        for path in ["/api", "/api/me", "/files/zip", "/auth/login", "/auth/"] {
            assert!(uses_session(path), "{path}");
        }
        for path in [
            "/",
            "/index.html",
            "/assets/app.js",
            "/healthz",
            "/apix",
            "/filesystem",
        ] {
            assert!(!uses_session(path), "{path}");
        }
    }

    #[test]
    fn ttl_is_kept_when_reasonable() {
        let ttl = Duration::from_hours(7 * 24);
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
        let lifetime = Lifetime::new(Duration::from_hours(7 * 24));
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
        let lifetime = Lifetime::new(Duration::from_hours(7 * 24));
        let (active, now) = stamps(30, 0);
        assert_eq!(lifetime.verdict(active, now), Verdict::End);
        let (almost, now) = stamps(29, 0);
        assert_eq!(lifetime.verdict(almost, now), Verdict::Keep);
    }

    #[test]
    fn expiry_never_passes_the_maximum_lifetime() {
        let lifetime = Lifetime::new(Duration::from_hours(7 * 24));
        let (early, now) = stamps(1, 0);
        assert_eq!(lifetime.remaining(early, now), time::Duration::days(7));
        let (late, now) = stamps(28, 0);
        assert_eq!(lifetime.remaining(late, now), time::Duration::days(2));
        let (over, now) = stamps(31, 0);
        assert_eq!(lifetime.remaining(over, now), time::Duration::ZERO);
    }

    #[test]
    fn maximum_lifetime_is_at_least_the_ttl() {
        let lifetime = Lifetime::new(Duration::from_hours(90 * 24));
        assert_eq!(lifetime.max, time::Duration::days(90));
        let lifetime = Lifetime::new(Duration::from_hours(24));
        assert_eq!(lifetime.max, time::Duration::days(30));
    }

    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;

        fn make_writer(&'a self) -> Captured {
            self.clone()
        }
    }

    async fn logged_when(task: tokio::task::JoinHandle<()>) -> String {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        watch(task).await.unwrap();
        String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
    }

    #[tokio::test]
    async fn a_panicking_cleanup_is_logged_at_error() {
        let log = logged_when(tokio::spawn(async { panic!("boom") })).await;
        assert!(log.contains("ERROR"), "{log}");
        assert!(log.contains("crashed"), "{log}");
    }

    #[tokio::test]
    async fn a_finished_cleanup_is_logged_at_error() {
        let log = logged_when(tokio::spawn(async {})).await;
        assert!(log.contains("ERROR"), "{log}");
        assert!(log.contains("stopped"), "{log}");
    }
}
