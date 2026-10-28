#![allow(dead_code)]

pub mod client;
pub mod fake_oidc;

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use explorer_server::auth::oidc::Oidc;
use explorer_server::config::Config;
use explorer_server::db;
use explorer_server::http::{AppState, router};
use explorer_server::storage::Volumes;
use explorer_server::tasks::Tasks;
use explorer_server::uploads::Uploads;
use fake_oidc::FakeOidc;

pub const PUBLIC_URL: &str = "https://files.example.com";
pub const ALLOWED_GROUP: &str = "explorer-users";

/// `dir` serves as config and web dir; `<dir>/data` is the data dir.
pub fn config(dir: &str, issuer: &str) -> Config {
    let data_dir = format!("{dir}/data");
    Config::from_lookup(|key| {
        let value = match key {
            "EXPLORER_PUBLIC_URL" => PUBLIC_URL,
            "EXPLORER_OIDC_ISSUER" => issuer,
            "EXPLORER_OIDC_CLIENT_ID" => fake_oidc::CLIENT_ID,
            "EXPLORER_OIDC_CLIENT_SECRET" => fake_oidc::CLIENT_SECRET,
            "EXPLORER_ALLOWED_GROUP" => ALLOWED_GROUP,
            "EXPLORER_DATA_DIR" => &data_dir,
            "EXPLORER_CONFIG_DIR" | "EXPLORER_WEB_DIR" => dir,
            _ => return None,
        };
        Some(value.to_owned())
    })
    .unwrap()
}

/// State wired to a running fake provider, which must outlive any sign-in exercised against it.
/// Creates the `media` volume under `<web_dir>/data`.
pub async fn state_with_oidc(web_dir: &Path) -> (AppState, FakeOidc) {
    state_with_volumes(web_dir, &["media"]).await
}

/// Like [`state_with_oidc`], with the given volumes under `<web_dir>/data`.
pub async fn state_with_volumes(web_dir: &Path, volumes: &[&str]) -> (AppState, FakeOidc) {
    let fake = FakeOidc::start().await;
    let config = config(web_dir.to_str().unwrap(), &fake.issuer());
    for name in volumes {
        std::fs::create_dir_all(config.data_dir.join(name)).unwrap();
    }
    let volumes = Volumes::discover(&config.data_dir).unwrap();
    let oidc = Oidc::discover(&config.oidc, &config.public_url)
        .await
        .unwrap();
    let db = db::open(web_dir).await.unwrap();
    let state = AppState {
        uploads: Arc::new(Uploads::new(db.clone())),
        db,
        config: Arc::new(config),
        oidc: Arc::new(oidc),
        volumes: Arc::new(volumes),
        tasks: Arc::new(Tasks::new()),
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
