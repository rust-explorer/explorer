mod cache;
mod render;
mod routes;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use tokio::task::JoinHandle;

use crate::http::AppState;

pub use cache::{Thumbnails, cache_key};
pub use render::{RenderError, Thumbnail, render};

/// Longest side of a thumbnail; the aspect ratio is kept.
pub const THUMB_SIZE: u32 = 320;
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PIXELS_SIDE: u32 = 16_384;
pub const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;
pub const THUMB_CONCURRENCY: usize = 2;
pub const THUMB_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// Part of the cache key: bump it when the rendering changes.
pub const THUMB_VERSION: u32 = 1;
/// Under the config directory.
pub const CACHE_DIR: &str = "cache/thumbnails";
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// Whether `name` is an image format thumbnails are made for.
#[must_use]
pub fn supported(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && ["jpg", "jpeg", "png", "gif", "webp"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/thumbnail", get(routes::thumbnail))
}

/// Sweeps the cache now, then every [`SWEEP_INTERVAL`].
pub fn spawn_sweeper(thumbnails: Arc<Thumbnails>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            let thumbnails = Arc::clone(&thumbnails);
            match tokio::task::spawn_blocking(move || thumbnails.sweep()).await {
                Ok(Ok(removed)) => tracing::debug!(removed, "thumbnail cache swept"),
                Ok(Err(err)) => tracing::warn!(error = %err, "cannot sweep the thumbnail cache"),
                Err(err) => tracing::warn!(error = %err, "thumbnail sweep failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_names() {
        for (name, expected) in [
            ("a.jpg", true),
            ("a.JPEG", true),
            ("b.Png", true),
            ("c.gif", true),
            ("d.webp", true),
            ("photo.tar.jpg", true),
            ("e.heic", false),
            ("f.svg", false),
            ("g.txt", false),
            ("jpg", false),
            (".jpg", false),
            ("", false),
        ] {
            assert_eq!(supported(name), expected, "{name}");
        }
    }
}
