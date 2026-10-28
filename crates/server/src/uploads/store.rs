use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use shared::api::ConflictPolicy;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, SqlitePool};

use crate::storage::{RelPath, UploadId};

/// An upload with no activity for this long is abandoned and swept.
pub const UPLOAD_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, PartialEq)]
pub struct UploadRecord {
    pub id: UploadId,
    pub user_sub: String,
    pub volume: String,
    pub path: RelPath,
    pub length: u64,
    pub modified_ms: Option<i64>,
    pub conflict: ConflictPolicy,
    /// Set once the upload is committed.
    pub final_path: Option<RelPath>,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLUMNS: &str = "id, user_sub, volume, path, length, modified_ms, conflict, final_path, \
                       created_at, updated_at";

pub struct Uploads {
    pool: SqlitePool,
    locks: Arc<Mutex<HashSet<UploadId>>>,
}

/// Held while a request works on an upload; released on drop.
pub struct UploadLock {
    id: UploadId,
    locks: Arc<Mutex<HashSet<UploadId>>>,
}

impl Drop for UploadLock {
    fn drop(&mut self) {
        self.locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
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
        id: UploadId::parse(&id).ok_or_else(|| corrupt("id"))?,
        user_sub: row.try_get("user_sub")?,
        volume: row.try_get("volume")?,
        path: RelPath::parse(&path).map_err(|_| corrupt("path"))?,
        length: u64::try_from(length).map_err(|_| corrupt("length"))?,
        modified_ms: row.try_get("modified_ms")?,
        conflict: ConflictPolicy::parse(&conflict).ok_or_else(|| corrupt("conflict"))?,
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
        }
    }

    pub async fn insert(&self, record: &UploadRecord) -> sqlx::Result<()> {
        let length = i64::try_from(record.length).map_err(|_| corrupt("length"))?;
        sqlx::query(&format!(
            "INSERT INTO uploads ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(record.id.as_str())
        .bind(&record.user_sub)
        .bind(&record.volume)
        .bind(record.path.as_str())
        .bind(length)
        .bind(record.modified_ms)
        .bind(record.conflict.as_str())
        .bind(record.final_path.as_ref().map(RelPath::as_str))
        .bind(record.created_at)
        .bind(record.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Another user's upload is reported as missing.
    pub async fn get(&self, id: &UploadId, user_sub: &str) -> sqlx::Result<Option<UploadRecord>> {
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

    pub async fn touch(&self, id: &UploadId, now_ms: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE uploads SET updated_at = ? WHERE id = ?")
            .bind(now_ms)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_done(
        &self,
        id: &UploadId,
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

    pub async fn delete(&self, id: &UploadId) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM uploads WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `None` while another request holds the upload.
    #[must_use]
    pub fn try_lock(&self, id: &UploadId) -> Option<UploadLock> {
        let inserted = self
            .locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone());
        inserted.then(|| UploadLock {
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

    pub(super) async fn exists(&self, id: &UploadId) -> sqlx::Result<bool> {
        Ok(sqlx::query("SELECT 1 FROM uploads WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?
            .is_some())
    }
}
