use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;

use explorer_server::app::{AppState, router};
use explorer_server::config::Config;
use explorer_server::db;
use explorer_server::healthcheck;
use explorer_server::telemetry;
use explorer_server::web::check_bundle;
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

const LISTEN_ADDR: &str = "0.0.0.0:8080";
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

    match serve(config, pool).await {
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

async fn serve(config: Config, db: SqlitePool) -> std::io::Result<()> {
    let app = router(AppState {
        config: Arc::new(config),
        db,
    });
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
