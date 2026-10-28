use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::{Row as _, SqlitePool};

/// Favorites one user can have.
pub const MAX_FAVORITES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    Added,
    AlreadyThere,
    Full,
}

pub struct Favorites {
    pool: SqlitePool,
}

impl Favorites {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// `(volume, path)` pairs in creation order.
    pub async fn list(&self, user: &str) -> sqlx::Result<Vec<(String, String)>> {
        sqlx::query(
            "SELECT volume, path FROM favorites WHERE user_sub = ? ORDER BY created_at, rowid",
        )
        .bind(user)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("volume")?, row.try_get("path")?)))
        .collect()
    }

    pub async fn add(&self, user: &str, volume: &str, path: &str) -> sqlx::Result<Added> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
            .unwrap_or_default();
        let limit = i64::try_from(MAX_FAVORITES).unwrap_or(i64::MAX);
        let inserted = sqlx::query(
            "INSERT INTO favorites (user_sub, volume, path, created_at) \
             SELECT ?1, ?2, ?3, ?4 \
             WHERE (SELECT COUNT(*) FROM favorites WHERE user_sub = ?1) < ?5 \
             ON CONFLICT DO NOTHING",
        )
        .bind(user)
        .bind(volume)
        .bind(path)
        .bind(now_ms)
        .bind(limit)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if inserted > 0 {
            return Ok(Added::Added);
        }
        let there =
            sqlx::query("SELECT 1 FROM favorites WHERE user_sub = ? AND volume = ? AND path = ?")
                .bind(user)
                .bind(volume)
                .bind(path)
                .fetch_optional(&self.pool)
                .await?
                .is_some();
        Ok(if there {
            Added::AlreadyThere
        } else {
            Added::Full
        })
    }

    /// Whether there was such a favorite.
    pub async fn remove(&self, user: &str, volume: &str, path: &str) -> sqlx::Result<bool> {
        let removed =
            sqlx::query("DELETE FROM favorites WHERE user_sub = ? AND volume = ? AND path = ?")
                .bind(user)
                .bind(volume)
                .bind(path)
                .execute(&self.pool)
                .await?
                .rows_affected();
        Ok(removed > 0)
    }
}
