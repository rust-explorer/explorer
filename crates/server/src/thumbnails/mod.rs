mod cache;
mod render;
mod routes;

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
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
pub const THUMB_MAX_AGE: Duration = Duration::from_hours(30 * 24);
/// Part of the cache key: bump it when the rendering changes.
pub const THUMB_VERSION: u32 = 1;
/// Under the config directory.
pub const CACHE_DIR: &str = "cache/thumbnails";
pub const SWEEP_INTERVAL: Duration = Duration::from_hours(24);

pub fn routes() -> Router<AppState> {
    Router::new().route("/thumbnail", get(routes::thumbnail))
}

/// Sweeps the cache now, then every [`SWEEP_INTERVAL`].
#[must_use = "dropping the handle detaches the task"]
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

/// Opens the cache under `config_dir`, creating its folders owner-only, and tightens the ones an
/// earlier version created with the default mode.
pub fn open_cache(config_dir: &Path) -> io::Result<Thumbnails> {
    let cache = Thumbnails::new(config_dir.join(CACHE_DIR))?;
    let mut dir = config_dir.to_owned();
    for name in Path::new(CACHE_DIR) {
        dir.push(name);
        let mode = std::fs::metadata(&dir)?.permissions().mode() & 0o777;
        if mode != 0o700
            && let Err(err) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        {
            tracing::warn!(path = %dir.display(), error = %err, "cannot make the thumbnail cache owner-only");
        }
    }
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn cache_folders_become_owner_only() {
        let config = tempfile::tempdir().unwrap();
        let thumbs = config.path().join(CACHE_DIR);
        std::fs::create_dir_all(&thumbs).unwrap();
        for dir in [config.path().join("cache"), thumbs.clone()] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        open_cache(config.path()).unwrap();

        assert_eq!(mode(&config.path().join("cache")), 0o700);
        assert_eq!(mode(&thumbs), 0o700);
    }
}
