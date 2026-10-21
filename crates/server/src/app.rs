use std::sync::Arc;

use axum::extract::Request as AxumRequest;
use axum::http::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{any, get};
use axum::{Json, Router};
use shared::api::Health;
use sqlx::SqlitePool;
use tower::ServiceBuilder;
use tower_http::LatencyUnit;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::config::Config;
use crate::error::{ApiError, attach_request_id};
use crate::oidc::Oidc;
use crate::{csrf, session, web};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub oidc: Arc<Oidc>,
}

pub async fn router(state: AppState) -> Result<Router, sqlx::Error> {
    router_with(state, Router::new()).await
}

/// Like [`router`], with `extra` routes served inside the same middleware stack.
pub async fn router_with(state: AppState, extra: Router<AppState>) -> Result<Router, sqlx::Error> {
    let trace = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<_>| {
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
        })
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Millis),
        );

    let session = session::layer(state.db.clone(), state.config.session_ttl).await?;
    let origin: Arc<str> = state
        .config
        .public_url
        .origin()
        .ascii_serialization()
        .into();

    let router = extra
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
                .layer(middleware::from_fn(drop_invalid_request_id))
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(trace)
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(middleware::from_fn(attach_request_id))
                .layer(middleware::from_fn_with_state(
                    origin,
                    csrf::require_same_origin,
                ))
                .layer(session),
        );
    Ok(web::security_headers(router))
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

const MAX_REQUEST_ID_LEN: usize = 128;

fn is_valid_request_id(id: &[u8]) -> bool {
    (1..=MAX_REQUEST_ID_LEN).contains(&id.len())
        && id
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Removes a client-supplied request id that fails validation so `SetRequestIdLayer` generates one.
async fn drop_invalid_request_id(mut request: AxumRequest, next: Next) -> Response {
    let invalid = request
        .headers()
        .get("x-request-id")
        .is_some_and(|v| !is_valid_request_id(v.as_bytes()));
    if invalid {
        request.headers_mut().remove("x-request-id");
    }
    next.run(request).await
}
