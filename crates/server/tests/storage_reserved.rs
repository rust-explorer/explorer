//! The staging directory stays out of reach whatever names and symlinks lead to it, even while
//! they change.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use explorer_server::storage::{RelPath, STAGING_DIR, StagedId, StorageError, TreeItem, Volumes};
use shared::api::conflict::ConflictPolicy;

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

#[test]
fn a_case_variant_of_the_staging_name_is_reserved() {
    let data = tempfile::tempdir().unwrap();
    let media = data.path().join("media");
    fs::create_dir_all(media.join("Holidays")).unwrap();
    fs::write(media.join("Holidays/photo.jpg"), b"jpg").unwrap();
    let volumes = Volumes::discover(data.path()).unwrap();
    let volume = volumes.get("media").unwrap();

    for name in [".EXPLORER-UPLOADS", ".Explorer-Uploads"] {
        assert!(RelPath::parse(name).is_err(), "{name}");
        let renamed = volume.rename(&rel("Holidays"), name, ConflictPolicy::Fail);
        assert!(
            matches!(renamed, Err(StorageError::InvalidPath(_))),
            "{renamed:?}"
        );
    }
    // One made outside the app is never listed at the root.
    fs::create_dir(media.join(".Explorer-UPLOADS")).unwrap();
    let names: Vec<_> = volume
        .list(&RelPath::root())
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["Holidays"]);
    // Nor zipped.
    fs::write(media.join(".Explorer-UPLOADS/hidden.txt"), b"x").unwrap();
    let mut walk = volume.tree_walk(&RelPath::root(), None).unwrap();
    let mut zipped = Vec::new();
    while let Some(item) = walk.next(volume) {
        zipped.push(match item {
            TreeItem::Dir { name, .. }
            | TreeItem::File { name, .. }
            | TreeItem::Skipped { name, .. } => name,
        });
    }
    assert_eq!(zipped, ["Holidays", "Holidays/photo.jpg"]);
}

/// Flips `R` between a plain folder and a symlink to the volume root while `probe` runs, and
/// fails if `probe` ever reaches the staging area.
fn race(media: &Path, probe: impl Fn() -> bool + Sync) {
    fs::create_dir_all(media.join("plain")).unwrap();
    symlink(".", media.join("link")).unwrap();
    let stop = AtomicBool::new(false);
    let leaked = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                let _ = fs::rename(media.join("link"), media.join("R"));
                let _ = fs::rename(media.join("R"), media.join("link"));
                let _ = fs::rename(media.join("plain"), media.join("R"));
                let _ = fs::rename(media.join("R"), media.join("plain"));
            }
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut leaked = 0;
        while Instant::now() < deadline {
            if probe() {
                leaked += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        leaked
    });
    assert_eq!(leaked, 0, "the staging area was reached {leaked} times");
}

#[test]
fn a_racing_rename_never_reaches_the_staging_area() {
    let data = tempfile::tempdir().unwrap();
    let media = data.path().join("media");
    fs::create_dir(&media).unwrap();
    let volumes = Volumes::discover(data.path()).unwrap();
    let volume = volumes.get("media").unwrap();
    let staging = volume.staging().unwrap();
    let id = StagedId::random();
    staging
        .create(&id)
        .unwrap()
        .write_all(b"secret partial upload")
        .unwrap();
    let listed = rel(&format!("R/{STAGING_DIR}"));
    let staged = rel(&format!("R/{STAGING_DIR}/{}", id.as_str()));
    let made = rel(&format!("R/{STAGING_DIR}/made"));

    race(&media, || {
        let listed = volume.list(&listed).is_ok();
        let read = volume.open_file(&staged).is_ok_and(|(mut file, _)| {
            let mut contents = String::new();
            file.read_to_string(&mut contents).is_ok() && contents.contains("secret")
        });
        let stat = volume.stat(&staged).is_ok();
        let created = volume.create_dir(&made).is_ok();
        let deleted = volume.delete(&staged).is_ok();
        listed || read || stat || created || deleted
    });
    assert!(media.join(STAGING_DIR).join(id.as_str()).is_file());
    assert!(!media.join(STAGING_DIR).join("made").exists());
}

/// A plain folder may hold its own `.explorer-uploads`, which has no `<id>`: missing there, the
/// path must not be looked up again from the root once `R` leads to it.
#[test]
fn a_missing_name_is_not_resolved_again_after_a_racing_rename() {
    let data = tempfile::tempdir().unwrap();
    let media = data.path().join("media");
    fs::create_dir_all(media.join("plain").join(STAGING_DIR)).unwrap();
    let volumes = Volumes::discover(data.path()).unwrap();
    let volume = volumes.get("media").unwrap();
    let id = StagedId::random();
    volume
        .staging()
        .unwrap()
        .create(&id)
        .unwrap()
        .write_all(b"secret partial upload")
        .unwrap();
    let staged = rel(&format!("R/{STAGING_DIR}/{}", id.as_str()));

    race(&media, || {
        volume.stat(&staged).is_ok() || volume.open_file(&staged).is_ok()
    });
}
