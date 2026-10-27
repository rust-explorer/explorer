use std::sync::Arc;

use serde_json::json;
use shared::api::conflict::ConflictPolicy;

use super::{Progress, TaskError};
use crate::http::error::ApiError;
use crate::storage::{self, EntryKind, MoveOutcome, RelPath, StorageError, Volume, Volumes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransferKind {
    Copy,
    Move,
}

impl TransferKind {
    fn action(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

/// A checked copy or move of `path` of the `source` volume into the folder `dest_dir` of the
/// `dest` volume.
pub(super) struct TransferTask {
    pub kind: TransferKind,
    pub source: String,
    pub path: RelPath,
    pub dest: String,
    pub dest_dir: RelPath,
    pub conflict: ConflictPolicy,
}

/// What a finished transfer logs: files, bytes and skipped entries for a copy.
type Done = (RelPath, Option<(u64, u64, u64)>);

fn source_and_dest<'a>(
    volumes: &'a Volumes,
    source: &str,
    dest: &str,
) -> Result<(&'a Volume, &'a Volume), ApiError> {
    let get = |name: &str| {
        volumes
            .get(name)
            .ok_or_else(|| ApiError::volume_not_found(name))
    };
    Ok((get(source)?, get(dest)?))
}

impl TransferTask {
    /// Refuses the transfer as the task would fail at once. An existing target answers 409 with
    /// its kind, so that the client knows whether it may offer to replace it.
    pub async fn check(&self, volumes: &Arc<Volumes>) -> Result<(), ApiError> {
        let volumes = Arc::clone(volumes);
        let (kind, from, to) = (self.kind, self.source.clone(), self.dest.clone());
        let (path, dir, conflict) = (self.path.clone(), self.dest_dir.clone(), self.conflict);
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let (source, dest) = source_and_dest(&volumes, &from, &to)?;
            let checked = match kind {
                TransferKind::Copy => storage::check_transfer(source, &path, dest, &dir, conflict),
                TransferKind::Move => storage::check_move(source, &path, dest, &dir, conflict),
            };
            match checked {
                Ok(()) => Ok(()),
                Err(StorageError::AlreadyExists(target)) => {
                    let kind = match dest.stat(&target) {
                        Ok(meta) if meta.kind == EntryKind::Dir => "dir",
                        _ => "file",
                    };
                    let details = json!({ "volume": to, "path": target.as_str(), "kind": kind });
                    Err(
                        ApiError::from_storage(&to, StorageError::AlreadyExists(target))
                            .with_details(details),
                    )
                }
                Err(err) => Err(ApiError::from_transfer(&from, &to, err)),
            }
        })
        .await
        .map_err(ApiError::internal)?
    }

    pub fn op(
        self,
        volumes: Arc<Volumes>,
        user: String,
    ) -> impl FnOnce(&Progress) -> Result<(), TaskError> + Send + 'static {
        move |progress| {
            let (from, to) = source_and_dest(&volumes, &self.source, &self.dest)?;
            let result = match self.kind {
                TransferKind::Copy => self.copy(from, to, progress),
                TransferKind::Move => self.move_entry(from, to, progress),
            };
            self.report(&user, result, progress)
        }
    }

    fn copy(&self, from: &Volume, to: &Volume, progress: &Progress) -> Result<Done, StorageError> {
        let plan = storage::plan_transfer(from, &self.path)?;
        progress.set_files_total(plan.files);
        progress.set_bytes_total(plan.bytes);
        let placed = storage::copy(
            from,
            &self.path,
            to,
            &self.dest_dir,
            self.conflict,
            &plan,
            progress,
        )?;
        Ok((placed, Some((plan.files, plan.bytes, plan.skipped))))
    }

    /// Plans only across volumes; a same-volume rename that meets a mount point plans itself.
    fn move_entry(
        &self,
        from: &Volume,
        to: &Volume,
        progress: &Progress,
    ) -> Result<Done, StorageError> {
        let plan = if from.name() == to.name() {
            None
        } else {
            let plan = storage::plan_transfer(from, &self.path)?;
            progress.set_files_total(plan.files);
            progress.set_bytes_total(plan.bytes);
            Some(plan)
        };
        let outcome = storage::move_entry(
            from,
            &self.path,
            to,
            &self.dest_dir,
            self.conflict,
            plan.as_ref(),
            progress,
        )?;
        Ok(match outcome {
            MoveOutcome::Renamed(placed) => {
                progress.set_files_total(1);
                progress.add_file();
                (placed, None)
            }
            MoveOutcome::Copied { final_path } => (final_path, Some(progress.counts())),
        })
    }

    fn report(
        &self,
        user: &str,
        result: Result<Done, StorageError>,
        progress: &Progress,
    ) -> Result<(), TaskError> {
        let (action, volume, path) = (self.kind.action(), &self.source, self.path.as_str());
        let dest_volume = &self.dest;
        match result {
            Ok((placed, counts)) => {
                progress.set_final_path(placed.as_str().to_owned());
                let final_path = placed.as_str();
                if let Some((files, bytes, skipped)) = counts {
                    tracing::info!(
                        user,
                        action,
                        volume,
                        path,
                        dest_volume,
                        final_path,
                        files,
                        bytes,
                        skipped,
                        "entry transferred"
                    );
                } else {
                    tracing::info!(
                        user,
                        action,
                        volume,
                        path,
                        dest_volume,
                        final_path,
                        "entry transferred"
                    );
                }
                Ok(())
            }
            Err(StorageError::Cancelled) => {
                tracing::info!(
                    user,
                    action,
                    volume,
                    path,
                    dest_volume,
                    "transfer cancelled"
                );
                Err(TaskError::Cancelled)
            }
            Err(err) => {
                tracing::warn!(
                    user,
                    action,
                    volume,
                    path,
                    dest_volume,
                    error = %err,
                    "transfer failed"
                );
                Err(ApiError::from_transfer(volume, dest_volume, err).into())
            }
        }
    }
}
