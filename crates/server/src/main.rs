use std::process::ExitCode;
use std::sync::Arc;

use explorer_server::app::{AppState, router};
use explorer_server::config::Config;
use explorer_server::telemetry;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};

const LISTEN_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() -> ExitCode {
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

    match serve(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "server failed");
            ExitCode::FAILURE
        }
    }
}

async fn serve(config: Config) -> std::io::Result<()> {
    let app = router(AppState {
        config: Arc::new(config),
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
