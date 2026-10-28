use shared::api::uploads::MAX_PENDING_UPLOADS;

use crate::http::AppState;
use crate::http::error::ApiError;
use crate::storage::{RelPath, StagedId, Volume, Volumes};

/// Refuses a new upload to a user who already has [`MAX_PENDING_UPLOADS`] unfinished ones.
pub(super) async fn check_pending_count(state: &AppState, user_sub: &str) -> Result<(), ApiError> {
    let pending = state
        .uploads
        .count_pending(user_sub)
        .await
        .map_err(ApiError::internal)?;
    if usize::try_from(pending).unwrap_or(usize::MAX) >= MAX_PENDING_UPLOADS {
        tracing::warn!(user = user_sub, pending, "too many unfinished uploads");
        return Err(ApiError::too_many_uploads());
    }
    Ok(())
}

/// Unfinished uploads as `(volume, id, length)`.
pub(super) type Pending = [(String, StagedId, u64)];

/// Refuses `length` bytes when the free space, less what the unfinished uploads to the volumes
/// on the same filesystem still have to write, cannot hold them. Runs on a blocking thread.
pub(super) fn check_space(
    volumes: &Volumes,
    vol: &Volume,
    path: &RelPath,
    length: u64,
    pending: &Pending,
) -> Result<(), ApiError> {
    let space = match vol.space() {
        Ok(space) => space,
        Err(err) => {
            tracing::warn!(volume = vol.name(), error = %err, "cannot read free space");
            return Ok(());
        }
    };
    let owed = volumes
        .iter()
        .map(|other| {
            let theirs: Vec<_> = pending
                .iter()
                .filter(|(volume, ..)| volume == other.name())
                .map(|(_, id, length)| (id, *length))
                .collect();
            if theirs.is_empty() {
                0
            } else {
                owed(other, &theirs)
            }
        })
        .fold(0, u64::saturating_add);
    let free = space.available.saturating_sub(owed);
    if length > free {
        tracing::warn!(
            volume = vol.name(),
            path = path.as_str(),
            length,
            available = space.available,
            owed,
            "upload larger than the free space"
        );
        return Err(ApiError::insufficient_storage(vol.name(), path.as_str()));
    }
    Ok(())
}

/// Bytes the pending uploads still have to write; one whose staged file is gone owes nothing.
fn owed(vol: &Volume, pending: &[(&StagedId, u64)]) -> u64 {
    let staging = match vol.existing_staging() {
        Ok(Some(staging)) => staging,
        Ok(None) => return 0,
        Err(err) => {
            tracing::warn!(volume = vol.name(), error = %err, "cannot open the staging directory");
            return 0;
        }
    };
    pending
        .iter()
        .filter_map(|(id, length)| {
            let staged = staging.len(id).ok()??;
            Some(length.saturating_sub(staged))
        })
        .fold(0, u64::saturating_add)
}
