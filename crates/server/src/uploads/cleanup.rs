use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::task::JoinHandle;

use super::store::{UPLOAD_TTL, UploadRecord, Uploads};
use crate::storage::{StagedId, Volumes};

pub const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    pub uploads: usize,
    pub orphans: usize,
}

/// Deletes uploads idle for longer than [`UPLOAD_TTL`] with their staged files, then staged
/// files that have no record and are just as old. Uploads locked by a request are left alone.
/// Failures are logged and skipped.
pub async fn sweep(uploads: &Uploads, volumes: &Arc<Volumes>, now: SystemTime) -> SweepReport {
    let mut report = SweepReport::default();
    let cutoff = now.checked_sub(UPLOAD_TTL).unwrap_or(UNIX_EPOCH);
    let cutoff_ms = i64::try_from(
        cutoff
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX);

    match uploads.expired(cutoff_ms).await {
        Ok(expired) => {
            for record in expired {
                if remove_expired(uploads, volumes, &record, cutoff_ms).await {
                    report.uploads += 1;
                }
            }
        }
        Err(err) => tracing::warn!(error = %err, "cannot list expired uploads"),
    }

    for name in volumes.iter().map(|v| v.name().to_owned()) {
        for id in old_staged(volumes, &name, cutoff).await {
            match uploads.exists(&id).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "cannot look up upload record");
                    continue;
                }
            }
            match remove_staged(volumes, &name, &id).await {
                Ok(()) => report.orphans += 1,
                Err(err) => {
                    tracing::warn!(error = %err, volume = %name, "cannot remove orphaned upload");
                }
            }
        }
    }

    if report.uploads + report.orphans > 0 {
        tracing::info!(
            action = "sweep",
            uploads = report.uploads,
            orphans = report.orphans,
            "removed abandoned uploads"
        );
    }
    report
}

/// Removes one upload of the expired list, unless a request holds it or touched it since the
/// list was read. Returns whether it was removed.
async fn remove_expired(
    uploads: &Uploads,
    volumes: &Arc<Volumes>,
    record: &UploadRecord,
    cutoff_ms: i64,
) -> bool {
    // A request working on it keeps it alive until its next `touch`.
    let Some(_lock) = uploads.try_lock(&record.id) else {
        return false;
    };
    match uploads.idle_since(&record.id, cutoff_ms).await {
        Ok(true) => {}
        Ok(false) => return false,
        Err(err) => {
            tracing::warn!(error = %err, "cannot look up upload record");
            return false;
        }
    }
    if record.final_path.is_none()
        && let Err(err) = remove_staged(volumes, &record.volume, &record.id).await
    {
        tracing::warn!(error = %err, volume = %record.volume, "cannot remove abandoned upload");
        return false;
    }
    if let Err(err) = uploads.delete(&record.id).await {
        tracing::warn!(error = %err, "cannot delete upload record");
        return false;
    }
    if record.final_path.is_none() {
        tracing::info!(
            user = record.user_sub,
            action = "sweep",
            volume = record.volume,
            path = record.path.as_str(),
            "abandoned upload removed"
        );
    }
    true
}

/// A volume that is gone or has no staging directory has nothing to remove.
async fn remove_staged(
    volumes: &Arc<Volumes>,
    volume: &str,
    id: &StagedId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let volumes = Arc::clone(volumes);
    let volume = volume.to_owned();
    let id = id.clone();
    tokio::task::spawn_blocking(move || {
        let Some(volume) = volumes.get(&volume) else {
            return Ok(());
        };
        if let Some(staging) = volume.existing_staging()? {
            staging.remove(&id)?;
        }
        Ok(())
    })
    .await?
}

async fn old_staged(volumes: &Arc<Volumes>, volume: &str, cutoff: SystemTime) -> Vec<StagedId> {
    let volumes = Arc::clone(volumes);
    let name = volume.to_owned();
    let listed = tokio::task::spawn_blocking(move || {
        let Some(volume) = volumes.get(&name) else {
            return Ok(Vec::new());
        };
        match volume.existing_staging()? {
            Some(staging) => staging.files(),
            None => Ok(Vec::new()),
        }
    })
    .await;
    match listed {
        Ok(Ok(files)) => files
            .into_iter()
            .filter(|(_, modified)| *modified < cutoff)
            .filter_map(|(name, _)| StagedId::parse(&name))
            .collect(),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, volume, "cannot list staged uploads");
            Vec::new()
        }
        Err(err) => {
            tracing::warn!(error = %err, volume, "staging listing failed");
            Vec::new()
        }
    }
}

/// Sweeps at startup, then every [`SWEEP_INTERVAL`].
#[must_use = "dropping the handle detaches the task"]
pub fn spawn_sweeper(uploads: Arc<Uploads>, volumes: Arc<Volumes>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            sweep(&uploads, &volumes, SystemTime::now()).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use shared::api::conflict::ConflictPolicy;

    use super::*;
    use crate::storage::RelPath;

    #[tokio::test]
    async fn an_upload_touched_after_the_listing_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (config, data) = (dir.path().join("config"), dir.path().join("data"));
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir_all(data.join("media")).unwrap();
        let uploads = Uploads::new(crate::db::open(&config).await.unwrap());
        let volumes = Arc::new(Volumes::discover(&data).unwrap());
        let record = UploadRecord {
            id: StagedId::random(),
            user_sub: "alice".to_owned(),
            volume: "media".to_owned(),
            path: RelPath::parse("Film.mkv").unwrap(),
            length: 10,
            modified_ms: None,
            conflict: ConflictPolicy::Fail,
            fingerprint: None,
            final_path: None,
            created_at: 1,
            updated_at: 1,
        };
        uploads.insert(&record).await.unwrap();
        volumes
            .get("media")
            .unwrap()
            .staging()
            .unwrap()
            .create(&record.id)
            .unwrap();

        let cutoff_ms = 1000;
        let listed = uploads.expired(cutoff_ms).await.unwrap();
        assert_eq!(listed.len(), 1);
        // A PATCH resumes it between the listing and the sweep's lock.
        uploads.touch(&record.id, 2000).await.unwrap();

        assert!(!remove_expired(&uploads, &volumes, &listed[0], cutoff_ms).await);
        assert!(uploads.get(&record.id, "alice").await.unwrap().is_some());
        let staging = volumes.get("media").unwrap().existing_staging().unwrap();
        assert_eq!(staging.unwrap().len(&record.id).unwrap(), Some(0));
    }
}
