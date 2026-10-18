use std::io::{IsTerminal, stdout};

use tracing_subscriber::EnvFilter;

use crate::config::{LogConfig, LogFormat};

/// # Panics
/// If `log.filter` is not a valid filter; `Config` validates it beforehand.
pub fn init(log: &LogConfig) {
    let filter = EnvFilter::try_new(&log.filter).expect("log filter validated by Config");
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match log.format {
        LogFormat::Pretty => builder.with_ansi(stdout().is_terminal()).init(),
        LogFormat::Json => builder.json().init(),
    }
}
