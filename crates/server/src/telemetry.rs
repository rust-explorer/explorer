use std::io::{IsTerminal, stdout};

use axum::http::{HeaderMap, HeaderName};
use tracing_subscriber::EnvFilter;

use crate::config::{LogConfig, LogFormat};

const MAX_LOGGED_LEN: usize = 256;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

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

#[must_use]
pub fn request_id(headers: &HeaderMap) -> &str {
    headers
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

/// Caps client-controlled values before they reach the logs.
#[must_use]
pub fn truncate_for_log(value: &str) -> String {
    value.chars().take(MAX_LOGGED_LEN).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_value_is_truncated() {
        let long = "a".repeat(1000);
        assert_eq!(truncate_for_log(&long).len(), MAX_LOGGED_LEN);
    }

    #[test]
    fn short_value_is_kept() {
        assert_eq!(truncate_for_log("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn request_id_defaults_to_empty() {
        let mut headers = HeaderMap::new();
        assert_eq!(request_id(&headers), "");
        headers.insert("x-request-id", "abc".parse().unwrap());
        assert_eq!(request_id(&headers), "abc");
    }
}
