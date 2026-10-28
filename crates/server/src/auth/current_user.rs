use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::http::error::ApiError;

pub(super) const USER_KEY: &str = "user";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurrentUser {
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|(_, msg)| ApiError::internal(msg))?;
        session
            .get::<Self>(USER_KEY)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(ApiError::unauthenticated)
    }
}
