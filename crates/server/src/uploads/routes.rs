use axum::Json;
use axum::extract::State;
use shared::api::uploads::PendingUpload;

use super::tus::staged_len;
use crate::auth::CurrentUser;
use crate::http::AppState;
use crate::http::error::ApiError;

/// The current user's unfinished uploads; those whose staged file is gone are left out.
pub async fn pending(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<PendingUpload>>, ApiError> {
    let records = state
        .uploads
        .list_pending(&user.sub)
        .await
        .map_err(ApiError::internal)?;
    let mut pending = Vec::with_capacity(records.len());
    for record in records {
        let Ok(Some(offset)) = staged_len(&state, &record).await else {
            continue;
        };
        pending.push(PendingUpload {
            id: record.id.as_str().to_owned(),
            volume: record.volume,
            path: record.path.as_str().to_owned(),
            length: record.length,
            offset,
            modified_ms: record.modified_ms,
            conflict: record.conflict,
            fingerprint: record.fingerprint,
        });
    }
    Ok(Json(pending))
}
