use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use shared::api::downloads::{ZipSelection, ZipSelectionCreated};

use crate::auth::{Action, CurrentUser};
use crate::http::AppState;
use crate::http::access::authorize;
use crate::http::error::ApiError;

/// `POST /files/zip/selections`: stores the entries to zip, too many for a URL, and answers the
/// id that `GET /files/zip?selection=` then streams. Nothing is read from disk yet; the zip
/// checks everything again.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    body: Result<Json<ZipSelection>, JsonRejection>,
) -> Result<(StatusCode, Json<ZipSelectionCreated>), ApiError> {
    let Ok(Json(selection)) = body else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON selection is required",
        ));
    };
    let path = authorize(
        &state,
        &user,
        &selection.volume,
        &selection.path,
        Action::Read,
    )?;
    if selection.names.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A selection names at least one entry",
        ));
    }
    for name in &selection.names {
        path.join(name)?;
    }
    let names = selection.names.len();
    let id = state.downloads.remember(&user.sub, selection);
    tracing::debug!(user = user.sub, names, "zip selection stored");
    Ok((StatusCode::CREATED, Json(ZipSelectionCreated { id })))
}
