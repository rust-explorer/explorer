#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use explorer_server::app::{AppState, router};
use explorer_server::config::Config;

pub fn state_with_web_dir(web_dir: &Path) -> AppState {
    let web = web_dir.to_str().unwrap().to_owned();
    let config = Config::from_lookup(|key| {
        let value = match key {
            "EXPLORER_PUBLIC_URL" => "https://files.example.com",
            "EXPLORER_OIDC_ISSUER" => "https://id.example.com",
            "EXPLORER_OIDC_CLIENT_ID" => "explorer",
            "EXPLORER_OIDC_CLIENT_SECRET" => "secret",
            "EXPLORER_ALLOWED_GROUP" => "explorer-users",
            "EXPLORER_DATA_DIR" | "EXPLORER_CONFIG_DIR" | "EXPLORER_WEB_DIR" => &web,
            _ => return None,
        };
        Some(value.to_owned())
    })
    .unwrap();
    AppState {
        config: Arc::new(config),
    }
}

pub fn app_with_web_dir(web_dir: &Path) -> axum::Router {
    router(state_with_web_dir(web_dir))
}

pub fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}
