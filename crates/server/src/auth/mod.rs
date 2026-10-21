//! Sign-in through the OIDC provider, the session that carries the signed-in user, the CSRF
//! check and the per-volume policy.

pub mod csrf;
mod current_user;
pub mod oidc;
mod policy;
mod routes;
pub mod session;

use axum::Router;
use axum::routing::{get, post};

use crate::http::AppState;

pub use current_user::CurrentUser;
pub use policy::{Action, Policy};
pub use routes::{me, sanitize_return_to};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(routes::login))
        .route("/callback", get(routes::callback))
        .route("/logout", post(routes::logout))
}
