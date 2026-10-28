use crate::http::AppState;
use crate::http::access::blocking_volume;
use crate::http::error::ApiError;
use crate::storage::StorageError;
use crate::uploads::UploadRecord;

/// The length of the staged file; `None` when it, its staging directory or its volume is gone.
pub(crate) async fn staged_len(
    state: &AppState,
    record: &UploadRecord,
) -> Result<Option<u64>, ApiError> {
    if state.volumes.get(&record.volume).is_none() {
        return Ok(None);
    }
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        match vol.existing_staging()? {
            Some(staging) => staging.len(&id),
            None => Ok(None),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

pub(super) async fn open_staged(
    state: &AppState,
    record: &UploadRecord,
) -> Result<Option<(std::fs::File, u64)>, ApiError> {
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        let Some(staging) = vol.existing_staging()? else {
            return Ok(None);
        };
        match staging.open_append(&id) {
            Ok(opened) => Ok(Some(opened)),
            Err(StorageError::NotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

pub(super) async fn remove_staged(state: &AppState, record: &UploadRecord) -> Result<(), ApiError> {
    if state.volumes.get(&record.volume).is_none() {
        return Ok(());
    }
    let id = record.id.clone();
    blocking_volume(&state.volumes, &record.volume, move |vol| {
        match vol.existing_staging()? {
            Some(staging) => staging.remove(&id),
            None => Ok(()),
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&record.volume, err))
}

/// Drops a pending record whose staged file is gone.
pub(super) async fn forget(state: &AppState, record: &UploadRecord) {
    tracing::warn!(
        volume = record.volume,
        path = record.path.as_str(),
        "staged upload file is missing; dropping the upload"
    );
    if let Err(err) = state.uploads.delete(&record.id).await {
        tracing::warn!(error = %err, "cannot delete upload record");
    }
}
