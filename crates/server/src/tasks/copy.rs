use std::sync::Arc;

use serde_json::json;
use shared::api::conflict::ConflictPolicy;
use shared::api::tasks::Location;

use super::{Progress, TaskError};
use crate::http::error::ApiError;
use crate::storage::{self, EntryKind, RelPath, StorageError, Volume, Volumes};

/// A checked copy of `path` into the folder `dir`.
pub(super) struct Copy {
    pub source: Location,
    pub dest_dir: Location,
    pub path: RelPath,
    pub dir: RelPath,
    pub conflict: ConflictPolicy,
}

fn both<'a>(
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

impl Copy {
    /// Refuses the copy as the task would fail at once. An existing target answers 409 with
    /// its kind, so that the client knows whether it may offer to replace it.
    pub async fn check(&self, volumes: &Arc<Volumes>) -> Result<(), ApiError> {
        let volumes = Arc::clone(volumes);
        let (from, to) = (self.source.volume.clone(), self.dest_dir.volume.clone());
        let (path, dir, conflict) = (self.path.clone(), self.dir.clone(), self.conflict);
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _entered = span.enter();
            let (source, dest) = both(&volumes, &from, &to)?;
            match storage::check_transfer(source, &path, dest, &dir, conflict) {
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
            let Self {
                source,
                dest_dir,
                path,
                dir,
                conflict,
            } = self;
            let (from, to) = both(&volumes, &source.volume, &dest_dir.volume)?;
            let copied = storage::plan_transfer(from, &path).and_then(|plan| {
                progress.set_files_total(plan.files);
                progress.set_bytes_total(plan.bytes);
                storage::copy(from, &path, to, &dir, conflict, &plan, progress)
                    .map(|placed| (placed, plan))
            });
            match copied {
                Ok((placed, plan)) => {
                    progress.set_final_path(placed.as_str().to_owned());
                    tracing::info!(
                        user,
                        action = "copy",
                        volume = source.volume,
                        path = path.as_str(),
                        dest_volume = dest_dir.volume,
                        final_path = placed.as_str(),
                        files = plan.files,
                        bytes = plan.bytes,
                        skipped = plan.skipped,
                        "entry copied"
                    );
                    Ok(())
                }
                Err(StorageError::Cancelled) => {
                    tracing::info!(
                        user,
                        action = "copy",
                        volume = source.volume,
                        path = path.as_str(),
                        dest_volume = dest_dir.volume,
                        "copy cancelled"
                    );
                    Err(TaskError::Cancelled)
                }
                Err(err) => {
                    tracing::warn!(
                        user,
                        action = "copy",
                        volume = source.volume,
                        path = path.as_str(),
                        dest_volume = dest_dir.volume,
                        error = %err,
                        "copy failed"
                    );
                    Err(ApiError::from_transfer(&source.volume, &dest_dir.volume, err).into())
                }
            }
        }
    }
}
