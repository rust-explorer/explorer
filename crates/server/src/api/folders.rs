use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use shared::api::CreateFolder;

use crate::auth::Action;
use crate::auth::CurrentUser;
use crate::http::AppState;
use crate::http::access::{authorize, blocking_volume};
use crate::http::error::ApiError;
use crate::storage::InvalidPath;

/// Creates a folder and its missing parents; an existing folder is fine.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    volume: Result<Path<String>, PathRejection>,
    body: Result<Json<CreateFolder>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let (Ok(Path(volume)), Ok(Json(CreateFolder { path }))) = (volume, body) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "A JSON body with a path is required",
        ));
    };
    let path = authorize(&state, &user, &volume, &path, Action::Write)?;
    if path.is_root() {
        return Err(InvalidPath {
            reason: "empty path",
        }
        .into());
    }
    blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.create_dir_all(&path)
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;
    tracing::info!(
        user = user.sub,
        action = "create_folder",
        volume,
        path = path.as_str(),
        "folder created"
    );
    Ok(StatusCode::NO_CONTENT)
}
