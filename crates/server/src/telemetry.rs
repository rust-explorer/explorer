use tracing_subscriber::EnvFilter;

use crate::config::{LogConfig, LogFormat};

pub fn init(log: &LogConfig) {
    let filter = EnvFilter::try_new(&log.filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match log.format {
        LogFormat::Pretty => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}
