use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng as _;
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, SemaphorePermit};

use super::{THUMB_CONCURRENCY, THUMB_MAX_AGE, THUMB_VERSION, Thumbnail};

/// A cached thumbnail used after this long gets its mtime refreshed, so that `sweep` keeps it.
const TOUCH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
const FORMATS: [(&str, &str); 2] = [("jpg", "image/jpeg"), ("png", "image/png")];

/// The on-disk thumbnail cache, and the permits that bound concurrent renders.
pub struct Thumbnails {
    dir: PathBuf,
    permits: Semaphore,
}

impl Thumbnails {
    /// Creates `dir` when missing.
    pub fn new(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            permits: Semaphore::new(THUMB_CONCURRENCY),
        })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Waits for a render slot; `None` only if the semaphore were closed, which never happens.
    pub async fn permit(&self) -> Option<SemaphorePermit<'_>> {
        self.permits.acquire().await.ok()
    }

    /// The cached thumbnail for `key`, if any.
    pub fn cached(&self, key: &str) -> io::Result<Option<Thumbnail>> {
        for (extension, content_type) in FORMATS {
            let path = self.dir.join(format!("{key}.{extension}"));
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            touch_if_old(&path);
            return Ok(Some(Thumbnail {
                bytes,
                content_type,
            }));
        }
        Ok(None)
    }

    /// Writes a temporary file, then renames it into place.
    pub fn store(&self, key: &str, thumbnail: &Thumbnail) -> io::Result<()> {
        let extension = FORMATS
            .iter()
            .find(|(_, content_type)| *content_type == thumbnail.content_type)
            .map_or("jpg", |(extension, _)| extension);
        let suffix: u64 = rand::thread_rng().r#gen();
        let temp = self.dir.join(format!("{key}.tmp-{suffix:016x}"));
        let written = File::create_new(&temp).and_then(|mut file| file.write_all(&thumbnail.bytes));
        let renamed =
            written.and_then(|()| fs::rename(&temp, self.dir.join(format!("{key}.{extension}"))));
        if renamed.is_err() {
            let _ = fs::remove_file(&temp);
        }
        renamed
    }

    /// Removes the files untouched for [`THUMB_MAX_AGE`]; returns how many went.
    pub fn sweep(&self) -> io::Result<usize> {
        let cutoff = SystemTime::now()
            .checked_sub(THUMB_MAX_AGE)
            .unwrap_or(UNIX_EPOCH);
        let mut removed = 0;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() || meta.modified().is_ok_and(|time| time >= cutoff) {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        Ok(removed)
    }
}

/// Failures only make the thumbnail expire sooner.
fn touch_if_old(path: &Path) {
    let now = SystemTime::now();
    let old = fs::metadata(path)
        .and_then(|meta| meta.modified())
        .is_ok_and(|time| time + TOUCH_AFTER < now);
    if old && let Ok(file) = File::options().write(true).open(path) {
        let _ = file.set_modified(now);
    }
}

/// Hex SHA-256 of the volume, path, size, mtime and [`THUMB_VERSION`].
#[must_use]
pub fn cache_key(volume: &str, path: &str, size: u64, modified: Option<SystemTime>) -> String {
    let nanos = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let mut hasher = Sha256::new();
    for part in [
        volume.as_bytes(),
        path.as_bytes(),
        size.to_string().as_bytes(),
        nanos.to_string().as_bytes(),
    ] {
        hasher.update(part);
        hasher.update([0]);
    }
    hasher.update(THUMB_VERSION.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_mtime(path: &Path, time: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    fn thumbnail(byte: u8) -> Thumbnail {
        Thumbnail {
            bytes: vec![byte; 4],
            content_type: "image/png",
        }
    }

    #[test]
    fn sweep_removes_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(dir.path().join("cache/thumbnails")).unwrap();
        thumbnails.store("old", &thumbnail(1)).unwrap();
        thumbnails.store("fresh", &thumbnail(2)).unwrap();
        let old = thumbnails.dir().join("old.png");
        set_mtime(
            &old,
            SystemTime::now() - THUMB_MAX_AGE - Duration::from_secs(60),
        );

        assert_eq!(thumbnails.sweep().unwrap(), 1);
        assert!(!old.exists());
        assert_eq!(thumbnails.cached("fresh").unwrap(), Some(thumbnail(2)));
        assert_eq!(thumbnails.cached("old").unwrap(), None);
    }

    #[test]
    fn stored_thumbnails_are_found_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(dir.path().to_owned()).unwrap();
        let jpeg = Thumbnail {
            bytes: vec![1, 2, 3],
            content_type: "image/jpeg",
        };
        thumbnails.store("k", &jpeg).unwrap();
        assert!(dir.path().join("k.jpg").is_file());
        assert_eq!(thumbnails.cached("k").unwrap(), Some(jpeg));
        assert_eq!(thumbnails.cached("other").unwrap(), None);
    }

    #[test]
    fn old_hits_are_touched() {
        let dir = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(dir.path().to_owned()).unwrap();
        thumbnails.store("k", &thumbnail(1)).unwrap();
        let path = dir.path().join("k.png");
        let old = SystemTime::now() - TOUCH_AFTER - Duration::from_secs(60);
        set_mtime(&path, old);
        thumbnails.cached("k").unwrap();
        assert!(fs::metadata(&path).unwrap().modified().unwrap() > old);
    }

    #[test]
    fn keys_change_with_every_part() {
        let time = UNIX_EPOCH + Duration::new(1_700_000_000, 5);
        let base = cache_key("media", "a.jpg", 10, Some(time));
        assert_eq!(base.len(), 64);
        assert!(base.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(base, cache_key("media", "a.jpg", 10, Some(time)));
        for other in [
            cache_key("other", "a.jpg", 10, Some(time)),
            cache_key("media", "b.jpg", 10, Some(time)),
            cache_key("media", "a.jpg", 11, Some(time)),
            cache_key("media", "a.jpg", 10, Some(time + Duration::from_nanos(1))),
            cache_key("media", "a.jpg", 10, None),
        ] {
            assert_ne!(base, other);
        }
    }
}
