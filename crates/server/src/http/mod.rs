use std::sync::Arc;

use axum::http::Request;
use axum::middleware;
use axum::routing::{any, get};
use axum::{Json, Router};
use shared::api::health::Health;
use sqlx::SqlitePool;
use tower::ServiceBuilder;
use tower_http::LatencyUnit;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub mod access;
pub mod error;
pub mod request_id;
pub mod security_headers;
pub mod static_files;

use error::{ApiError, attach_request_id};
use request_id::{drop_invalid_request_id, request_id};

use crate::auth::oidc::Oidc;
use crate::auth::{self, csrf, session};
use crate::browse;
use crate::config::Config;
use crate::downloads::{self, Downloads};
use crate::favorites::{self, Favorites};
use crate::search;
use crate::storage::Volumes;
use crate::tasks::{self, Tasks};
use crate::thumbnails::{self, Thumbnails};
use crate::uploads::{self, Uploads};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub oidc: Arc<Oidc>,
    pub volumes: Arc<Volumes>,
    pub uploads: Arc<Uploads>,
    pub tasks: Arc<Tasks>,
    pub favorites: Arc<Favorites>,
    pub thumbnails: Arc<Thumbnails>,
    pub downloads: Arc<Downloads>,
}

pub async fn router(state: AppState) -> Result<Router, sqlx::Error> {
    router_with(state, Router::new()).await
}

/// Like [`router`], with `extra` routes served inside the same middleware stack.
pub async fn router_with(state: AppState, extra: Router<AppState>) -> Result<Router, sqlx::Error> {
    let trace = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
            let request_id = request_id(req.headers());
            tracing::info_span!(
                "request",
                method = %req.method(),
                path = %req.uri().path(),
                request_id,
            )
        })
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Millis),
        );

    let session = session::layer(state.db.clone()).await?;
    let keep_alive = session::KeepAlive::new(state.config.session_ttl, state.db.clone());
    let origin: Arc<str> = csrf::public_origin(&state.config.public_url).into();

    let router = extra
        .route("/healthz", get(healthz))
        .nest(
            "/api",
            Router::new()
                .route("/me", get(auth::me))
                .merge(browse::routes())
                .merge(uploads::routes())
                .merge(tasks::routes())
                .merge(favorites::routes())
                .merge(search::routes())
                .fallback(not_found)
                .layer(middleware::from_fn(csrf::refuse_cross_site)),
        )
        .nest("/auth", auth::routes().fallback(not_found))
        .nest(
            "/files",
            downloads::routes()
                .merge(thumbnails::routes())
                .nest("/uploads", uploads::tus_routes())
                .fallback(not_found)
                .layer(middleware::from_fn(csrf::refuse_cross_site)),
        )
        // nest() does not match the empty tail, so the bare-slash forms need explicit routes
        .route("/api/", any(not_found))
        .route("/auth/", any(not_found))
        .route("/files/", any(not_found))
        .fallback_service(static_files::service(&state.config.web_dir))
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(drop_invalid_request_id))
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(trace)
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(middleware::from_fn(attach_request_id))
                .layer(middleware::from_fn(uploads::tag_tus_responses))
                .layer(middleware::from_fn_with_state(
                    origin,
                    csrf::require_same_origin,
                ))
                .layer(session)
                .layer(middleware::from_fn_with_state(
                    keep_alive,
                    session::keep_alive,
                )),
        );
    Ok(security_headers::security_headers(router))
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}
