use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use shared::api::conflict::ConflictPolicy;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, SqlitePool};

use crate::storage::{RelPath, StagedId};

/// An upload with no activity for this long is abandoned and swept.
pub const UPLOAD_TTL: Duration = Duration::from_hours(24);

#[derive(Debug, Clone, PartialEq)]
pub struct UploadRecord {
    pub id: StagedId,
    pub user_sub: String,
    pub volume: String,
    pub path: RelPath,
    pub length: u64,
    pub modified_ms: Option<i64>,
    pub conflict: ConflictPolicy,
    /// The client's [`shared::tus::fingerprint`] of the file.
    pub fingerprint: Option<String>,
    /// Set once the upload is committed.
    pub final_path: Option<RelPath>,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLUMNS: &str = "id, user_sub, volume, path, length, modified_ms, conflict, fingerprint, \
                       final_path, created_at, updated_at";

/// Locked uploads, each with whether a cancel arrived while it was held.
type Locks = Arc<Mutex<HashMap<StagedId, bool>>>;
/// `PATCH` requests in flight per user.
type Patches = Arc<Mutex<HashMap<String, usize>>>;

pub struct Uploads {
    pool: SqlitePool,
    locks: Locks,
    admission: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    patches: Patches,
}

/// Held while a request works on an upload; released on drop.
pub struct UploadLock {
    id: StagedId,
    locks: Locks,
}

impl UploadLock {
    /// Whether a cancel arrived since the lock was taken; checked right before committing.
    #[must_use]
    pub fn cancel_requested(&self) -> bool {
        self.locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&self.id)
            .copied()
            .unwrap_or_default()
    }
}

impl Drop for UploadLock {
    fn drop(&mut self) {
        self.locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

/// One of a user's `PATCH` requests in flight; released on drop.
pub struct PatchSlot {
    user_sub: String,
    patches: Patches,
}

impl Drop for PatchSlot {
    fn drop(&mut self) {
        let mut patches = self.patches.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = patches.get_mut(&self.user_sub) {
            *count -= 1;
            if *count == 0 {
                patches.remove(&self.user_sub);
            }
        }
    }
}

fn corrupt(column: &str) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: column.to_owned(),
        source: format!("invalid value in `{column}`").into(),
    }
}

fn record(row: &SqliteRow) -> sqlx::Result<UploadRecord> {
    let path: String = row.try_get("path")?;
    let final_path: Option<String> = row.try_get("final_path")?;
    let length: i64 = row.try_get("length")?;
    let id: String = row.try_get("id")?;
    let conflict: String = row.try_get("conflict")?;
    Ok(UploadRecord {
        id: StagedId::parse(&id).ok_or_else(|| corrupt("id"))?,
        user_sub: row.try_get("user_sub")?,
        volume: row.try_get("volume")?,
        path: RelPath::parse(&path).map_err(|_| corrupt("path"))?,
        length: u64::try_from(length).map_err(|_| corrupt("length"))?,
        modified_ms: row.try_get("modified_ms")?,
        conflict: ConflictPolicy::parse(&conflict).ok_or_else(|| corrupt("conflict"))?,
        fingerprint: row.try_get("fingerprint")?,
        final_path: final_path
            .map(|p| RelPath::parse(&p).map_err(|_| corrupt("final_path")))
            .transpose()?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Uploads {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            locks: Arc::default(),
            admission: Mutex::default(),
            patches: Arc::default(),
        }
    }

    pub async fn insert(&self, record: &UploadRecord) -> sqlx::Result<()> {
        let length = i64::try_from(record.length).map_err(|_| corrupt("length"))?;
        sqlx::query(&format!(
            "INSERT INTO uploads ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(record.id.as_str())
        .bind(&record.user_sub)
        .bind(&record.volume)
        .bind(record.path.as_str())
        .bind(length)
        .bind(record.modified_ms)
        .bind(record.conflict.as_str())
        .bind(record.fingerprint.as_deref())
        .bind(record.final_path.as_ref().map(RelPath::as_str))
        .bind(record.created_at)
        .bind(record.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Another user's upload is reported as missing.
    pub async fn get(&self, id: &StagedId, user_sub: &str) -> sqlx::Result<Option<UploadRecord>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM uploads WHERE id = ? AND user_sub = ?"
        ))
        .bind(id.as_str())
        .bind(user_sub)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(record)
        .transpose()
    }

    /// Uploads not yet committed, oldest first.
    pub async fn list_pending(&self, user_sub: &str) -> sqlx::Result<Vec<UploadRecord>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM uploads WHERE user_sub = ? AND final_path IS NULL \
             ORDER BY created_at, id"
        ))
        .bind(user_sub)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(record)
        .collect()
    }

    /// Held from the checks of a new upload to `volume` until its record is inserted, so that
    /// concurrent creations on the volume see each other. One lock per volume: a hung volume only
    /// holds up uploads to itself.
    pub async fn admission(&self, volume: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = Arc::clone(
            self.admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(volume.to_owned())
                .or_default(),
        );
        lock.lock_owned().await
    }

    pub async fn count_pending(&self, user_sub: &str) -> sqlx::Result<u64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM uploads WHERE user_sub = ? AND final_path IS NULL",
        )
        .bind(user_sub)
        .fetch_one(&self.pool)
        .await?;
        Ok(u64::try_from(count).unwrap_or_default())
    }

    /// The volume, id and length of every unfinished upload to one of `volumes`, whoever started
    /// it.
    pub async fn pending_on(
        &self,
        volumes: &[String],
    ) -> sqlx::Result<Vec<(String, StagedId, u64)>> {
        if volumes.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; volumes.len()].join(", ");
        let sql = format!(
            "SELECT volume, id, length FROM uploads WHERE volume IN ({marks}) \
             AND final_path IS NULL"
        );
        let mut query = sqlx::query(&sql);
        for volume in volumes {
            query = query.bind(volume);
        }
        query
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let length: i64 = row.try_get("length")?;
                Ok((
                    row.try_get("volume")?,
                    StagedId::parse(&id).ok_or_else(|| corrupt("id"))?,
                    u64::try_from(length).map_err(|_| corrupt("length"))?,
                ))
            })
            .collect()
    }

    pub async fn touch(&self, id: &StagedId, now_ms: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE uploads SET updated_at = ? WHERE id = ?")
            .bind(now_ms)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_done(
        &self,
        id: &StagedId,
        final_path: &RelPath,
        now_ms: i64,
    ) -> sqlx::Result<()> {
        sqlx::query("UPDATE uploads SET final_path = ?, updated_at = ? WHERE id = ?")
            .bind(final_path.as_str())
            .bind(now_ms)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &StagedId) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM uploads WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `None` while the user already has `max` requests in flight.
    #[must_use]
    pub fn try_start_patch(&self, user_sub: &str, max: usize) -> Option<PatchSlot> {
        let mut patches = self.patches.lock().unwrap_or_else(PoisonError::into_inner);
        let count = patches.entry(user_sub.to_owned()).or_default();
        if *count >= max {
            return None;
        }
        *count += 1;
        Some(PatchSlot {
            user_sub: user_sub.to_owned(),
            patches: Arc::clone(&self.patches),
        })
    }

    /// `None` while another request holds the upload.
    #[must_use]
    pub fn try_lock(&self, id: &StagedId) -> Option<UploadLock> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if locks.contains_key(id) {
            return None;
        }
        locks.insert(id.clone(), false);
        Some(UploadLock {
            id: id.clone(),
            locks: Arc::clone(&self.locks),
        })
    }

    /// Like [`Uploads::try_lock`]; while another request holds the upload, also asks it not to
    /// commit.
    #[must_use]
    pub fn lock_or_request_cancel(&self, id: &StagedId) -> Option<UploadLock> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cancelled) = locks.get_mut(id) {
            *cancelled = true;
            return None;
        }
        locks.insert(id.clone(), false);
        Some(UploadLock {
            id: id.clone(),
            locks: Arc::clone(&self.locks),
        })
    }

    pub(super) async fn expired(&self, before_ms: i64) -> sqlx::Result<Vec<UploadRecord>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM uploads WHERE updated_at < ?"
        ))
        .bind(before_ms)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(record)
        .collect()
    }

    /// Whether the upload still exists and was last touched before `before_ms`.
    pub(super) async fn idle_since(&self, id: &StagedId, before_ms: i64) -> sqlx::Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM uploads WHERE id = ? AND updated_at < ?")
                .bind(id.as_str())
                .bind(before_ms)
                .fetch_optional(&self.pool)
                .await?
                .is_some(),
        )
    }

    pub(super) async fn exists(&self, id: &StagedId) -> sqlx::Result<bool> {
        Ok(sqlx::query("SELECT 1 FROM uploads WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .is_some())
    }
}
