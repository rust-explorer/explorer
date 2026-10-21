use axum::Json;
use axum::extract::State;
use shared::api::PendingUpload;

use crate::auth::CurrentUser;
use crate::http::AppState;
use crate::http::error::ApiError;
use crate::uploads::staged_len;

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
        });
    }
    Ok(Json(pending))
}
