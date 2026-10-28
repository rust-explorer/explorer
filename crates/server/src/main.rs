use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use explorer_server::auth::oidc::Oidc;
use explorer_server::auth::session;
use explorer_server::config::Config;
use explorer_server::db;
use explorer_server::favorites::Favorites;
use explorer_server::healthcheck;
use explorer_server::http::static_files::check_bundle;
use explorer_server::http::{AppState, router};
use explorer_server::storage::Volumes;
use explorer_server::tasks::Tasks;
use explorer_server::telemetry;
use explorer_server::thumbnails::{self, Thumbnails};
use explorer_server::uploads::{self, Uploads};
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

const LISTEN_ADDR: &str = "0.0.0.0:8080";
const DISCOVERY_ATTEMPTS: u32 = 10;
const DISCOVERY_DELAY: Duration = Duration::from_secs(3);
const HEALTHCHECK_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

#[tokio::main]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("healthcheck") => return run_healthcheck().await,
        Some(other) => {
            eprintln!("unknown argument `{other}`; usage: explorer [healthcheck]");
            return ExitCode::from(2);
        }
    }

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(errors) => {
            eprintln!("{errors}");
            return ExitCode::from(2);
        }
    };
    telemetry::init(&config.log);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        uid = rustix::process::getuid().as_raw(),
        gid = rustix::process::getgid().as_raw(),
        "starting explorer"
    );

    if let Err(err) = check_bundle(&config.web_dir) {
        tracing::error!(error = %err, "invalid web bundle");
        return ExitCode::from(2);
    }

    let pool = match db::open(&config.config_dir).await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, "database unavailable");
            return ExitCode::from(2);
        }
    };

    let volumes = match Volumes::discover(&config.data_dir) {
        Ok(volumes) => volumes,
        Err(err) => {
            tracing::error!(
                error = %err,
                dir = %config.data_dir.display(),
                "data directory unreadable"
            );
            return ExitCode::from(2);
        }
    };

    let oidc = match Oidc::discover_with_retry(
        &config.oidc,
        &config.public_url,
        DISCOVERY_ATTEMPTS,
        DISCOVERY_DELAY,
    )
    .await
    {
        Ok(oidc) => oidc,
        Err(err) => {
            tracing::error!(error = %err, "OIDC provider unavailable");
            return ExitCode::from(2);
        }
    };

    let thumbnail_cache = match Thumbnails::new(config.config_dir.join(thumbnails::CACHE_DIR)) {
        Ok(cache) => Arc::new(cache),
        Err(err) => {
            tracing::error!(error = %err, "thumbnail cache unavailable");
            return ExitCode::from(2);
        }
    };

    let volumes = Arc::new(volumes);
    let upload_registry = Arc::new(Uploads::new(pool.clone()));
    let state = AppState {
        config: Arc::new(config),
        db: pool.clone(),
        oidc: Arc::new(oidc),
        volumes: Arc::clone(&volumes),
        uploads: Arc::clone(&upload_registry),
        tasks: Arc::new(Tasks::new()),
        favorites: Arc::new(Favorites::new(pool.clone())),
        thumbnails: Arc::clone(&thumbnail_cache),
    };
    tracing::info!("no background task to recover: tasks live in memory only");
    let app = match router(state).await {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(error = %err, "session store unavailable");
            return ExitCode::from(2);
        }
    };
    let _cleanup = session::spawn_expired_cleanup(pool);
    let _sweeper = uploads::spawn_sweeper(upload_registry, volumes);
    let _thumbnail_sweeper = thumbnails::spawn_sweeper(thumbnail_cache);

    match serve(app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "server failed");
            ExitCode::FAILURE
        }
    }
}

async fn run_healthcheck() -> ExitCode {
    match healthcheck::run(HEALTHCHECK_ADDR).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("unhealthy: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(app: Router) -> std::io::Result<()> {
    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!(addr = LISTEN_ADDR, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn shutdown_signal() {
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    tracing::info!("shutting down");
}
