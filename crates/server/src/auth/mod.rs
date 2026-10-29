//! Sign-in through the OIDC provider, the session that carries the signed-in user, the CSRF
//! check and the per-volume policy.

pub mod csrf;
mod current_user;
pub mod oidc;
mod policy;
mod routes;
pub mod session;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};

use crate::http::AppState;
use crate::http::rate_limit::{SignInLimit, limit_sign_in};

pub use current_user::CurrentUser;
pub use policy::{Action, Policy};
pub use routes::me;

/// Sign-in routes are open to anyone, so they are limited per client address.
pub fn routes(sign_in_limit: SignInLimit) -> Router<AppState> {
    let limited = middleware::from_fn_with_state(sign_in_limit, limit_sign_in);
    Router::new()
        .route("/login", get(routes::login).layer(limited.clone()))
        .route("/callback", get(routes::callback).layer(limited))
        .route("/logout", post(routes::logout))
}
