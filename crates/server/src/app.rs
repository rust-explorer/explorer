use std::sync::Arc;

use axum::http::Request;
use axum::middleware;
use axum::routing::{any, get};
use axum::{Json, Router};
use shared::api::Health;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::error::{ApiError, attach_request_id};
use crate::web;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    let trace = TraceLayer::new_for_http().make_span_with(|req: &Request<_>| {
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        tracing::info_span!(
            "request",
            method = %req.method(),
            path = %req.uri().path(),
            request_id,
        )
    });

    let router = Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", Router::new().fallback(not_found))
        .nest("/auth", Router::new().fallback(not_found))
        // nest() does not match the empty tail, so the bare-slash forms need explicit routes
        .route("/api/", any(not_found))
        .route("/auth/", any(not_found))
        .fallback_service(web::service(&state.config.web_dir))
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(trace)
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(middleware::from_fn(attach_request_id)),
        );
    web::security_headers(router)
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
