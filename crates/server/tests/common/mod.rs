#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use explorer_server::app::{AppState, router};
use explorer_server::config::Config;
use explorer_server::db;
use explorer_server::oidc::Oidc;
use fake_oidc::FakeOidc;

pub const PUBLIC_URL: &str = "https://files.example.com";
pub const ALLOWED_GROUP: &str = "explorer-users";

pub fn config(web_dir: &str, issuer: &str) -> Config {
    Config::from_lookup(|key| {
        let value = match key {
            "EXPLORER_PUBLIC_URL" => PUBLIC_URL,
            "EXPLORER_OIDC_ISSUER" => issuer,
            "EXPLORER_OIDC_CLIENT_ID" => fake_oidc::CLIENT_ID,
            "EXPLORER_OIDC_CLIENT_SECRET" => fake_oidc::CLIENT_SECRET,
            "EXPLORER_ALLOWED_GROUP" => ALLOWED_GROUP,
            "EXPLORER_DATA_DIR" | "EXPLORER_CONFIG_DIR" | "EXPLORER_WEB_DIR" => web_dir,
            _ => return None,
        };
        Some(value.to_owned())
    })
    .unwrap()
}

/// State wired to a running fake provider, which must outlive any sign-in exercised against it.
pub async fn state_with_oidc(web_dir: &Path) -> (AppState, FakeOidc) {
    let fake = FakeOidc::start().await;
    let config = config(web_dir.to_str().unwrap(), &fake.issuer());
    let oidc = Oidc::discover(&config.oidc, &config.public_url)
        .await
        .unwrap();
    let state = AppState {
        db: db::open(web_dir).await.unwrap(),
        config: Arc::new(config),
        oidc: Arc::new(oidc),
    };
    (state, fake)
}

pub async fn state_with_web_dir(web_dir: &Path) -> AppState {
    state_with_oidc(web_dir).await.0
}

pub async fn app_with_web_dir(web_dir: &Path) -> axum::Router {
    router(state_with_web_dir(web_dir).await).await.unwrap()
}

pub fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}
