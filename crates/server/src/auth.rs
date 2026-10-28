use std::fmt::Write as _;

use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use shared::api::{LogoutResponse, Me};
use tower_sessions::Session;

use crate::app::AppState;
use crate::error::ApiError;
use crate::oidc::PendingLogin;

const PENDING_KEY: &str = "oidc_pending";
const USER_KEY: &str = "user";
const ID_TOKEN_KEY: &str = "id_token";
const MAX_RETURN_TO_LEN: usize = 2048;
const MAX_LOGGED_LEN: usize = 256;
const ACCESS_DENIED_PATH: &str = "/access-denied";

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

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(login))
        .route("/callback", get(callback))
        .route("/logout", post(logout))
}

pub async fn me(user: CurrentUser) -> Json<Me> {
    Json(Me {
        sub: user.sub,
        name: user.name,
        email: user.email,
    })
}

/// Keeps only same-origin absolute paths; anything that could leave the site becomes `/`.
#[must_use]
pub fn sanitize_return_to(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return "/".to_owned();
    };
    let mut chars = raw.chars();
    let acceptable = chars.next() == Some('/')
        && !matches!(chars.next(), Some('/' | '\\'))
        && raw.len() <= MAX_RETURN_TO_LEN
        && !raw.chars().any(char::is_control);
    if !acceptable {
        return "/".to_owned();
    }
    // Non-ASCII bytes are not valid in a `Location` header value.
    let mut sanitized = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii() {
            sanitized.push(char::from(byte));
        } else {
            let _ = write!(sanitized, "%{byte:02X}");
        }
    }
    sanitized
}

#[derive(Deserialize)]
struct LoginParams {
    return_to: Option<String>,
}

async fn login(
    State(state): State<AppState>,
    session: Session,
    params: Result<Query<LoginParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let return_to = params.ok().and_then(|Query(p)| p.return_to);
    let (url, mut pending) = state.oidc.authorize_url();
    pending.return_to = sanitize_return_to(return_to.as_deref());
    session
        .insert(PENDING_KEY, pending)
        .await
        .map_err(ApiError::internal)?;
    Ok(see_other(url.as_str()))
}

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    params: Result<Query<CallbackParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    // Taken before anything else so a state can be used at most once, whatever the outcome.
    let pending = session
        .remove::<PendingLogin>(PENDING_KEY)
        .await
        .map_err(ApiError::internal)?;
    let params = params.map(|Query(p)| p).ok();

    if let Some(error) = params.as_ref().and_then(|p| p.error.as_deref()) {
        tracing::warn!(
            request_id,
            reason = "provider_error",
            error = truncated(error),
            "sign-in failed"
        );
        return Ok(signin_error("provider_error"));
    }
    let Some((code, pending)) = params
        .and_then(|p| Some((p.code?, p.state?)))
        .zip(pending)
        .filter(|((_, got), pending)| *got == pending.csrf_state)
        .map(|((code, _), pending)| (code, pending))
    else {
        tracing::warn!(request_id, reason = "state_mismatch", "sign-in failed");
        return Ok(signin_error("state_mismatch"));
    };

    let return_to = pending.return_to.clone();
    let identity = match state.oidc.exchange(code, pending).await {
        Ok(identity) => identity,
        Err(err) => {
            tracing::warn!(
                request_id,
                reason = err.reason(),
                error = err.to_string(),
                "sign-in failed"
            );
            return Ok(signin_error(err.reason()));
        }
    };

    if !identity.groups.contains(&state.config.oidc.allowed_group) {
        tracing::info!(
            request_id,
            sub = identity.sub,
            groups = ?identity.groups,
            "access denied: not in the allowed group"
        );
        session.flush().await.map_err(ApiError::internal)?;
        return Ok(see_other(ACCESS_DENIED_PATH));
    }

    session.cycle_id().await.map_err(ApiError::internal)?;
    let user = CurrentUser {
        sub: identity.sub,
        name: identity.name,
        email: identity.email,
        groups: identity.groups,
    };
    session
        .insert(USER_KEY, &user)
        .await
        .map_err(ApiError::internal)?;
    session
        .insert(ID_TOKEN_KEY, identity.id_token)
        .await
        .map_err(ApiError::internal)?;
    tracing::info!(request_id, sub = user.sub, "signed in");
    Ok(see_other(&return_to))
}

async fn logout(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<LogoutResponse>, ApiError> {
    let id_token = session
        .get::<String>(ID_TOKEN_KEY)
        .await
        .map_err(ApiError::internal)?;
    session.flush().await.map_err(ApiError::internal)?;
    let redirect_to = id_token
        .and_then(|token| state.oidc.end_session_url(&token, &state.config.public_url))
        .map_or_else(|| "/".to_owned(), String::from);
    Ok(Json(LogoutResponse { redirect_to }))
}

fn signin_error(reason: &'static str) -> Response {
    see_other(&format!("/signin-error?reason={reason}"))
}

/// A 303 that falls back to `/` rather than panicking on a location unfit for a header.
fn see_other(location: &str) -> Response {
    let value = HeaderValue::try_from(location).unwrap_or(HeaderValue::from_static("/"));
    (StatusCode::SEE_OTHER, [(header::LOCATION, value)]).into_response()
}

fn request_id(headers: &HeaderMap) -> &str {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn truncated(value: &str) -> String {
    value.chars().take(MAX_LOGGED_LEN).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_return_to_rejects_open_redirects() {
        let long = format!("/{}", "a".repeat(3000));
        for raw in [
            "//evil.example",
            "/\\evil.example",
            "https://evil.example",
            "javascript:alert(1)",
            "",
            long.as_str(),
            "/ok\n",
            "/ok\t",
            "/ok\u{7f}",
            "evil.example",
        ] {
            assert_eq!(sanitize_return_to(Some(raw)), "/", "{raw:?}");
        }
        assert_eq!(sanitize_return_to(None), "/");
    }

    #[test]
    fn sanitize_return_to_keeps_local_paths() {
        for raw in ["/", "/docs/a?b=1", "/docs/a#frag", "/a/b\\c", "/a//b"] {
            assert_eq!(sanitize_return_to(Some(raw)), raw);
        }
        let max = format!("/{}", "a".repeat(MAX_RETURN_TO_LEN - 1));
        assert_eq!(sanitize_return_to(Some(&max)), max);
    }

    #[test]
    fn sanitize_return_to_percent_encodes_non_ascii() {
        assert_eq!(sanitize_return_to(Some("/caf\u{e9}")), "/caf%C3%A9");
        assert!(HeaderValue::try_from(sanitize_return_to(Some("/\u{1f600}"))).is_ok());
    }

    #[test]
    fn see_other_never_panics_on_bad_locations() {
        let res = see_other("/bad\nlocation");
        assert_eq!(res.headers()[header::LOCATION], "/");
    }
}
