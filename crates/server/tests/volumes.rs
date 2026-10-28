use std::fs;
use std::os::unix::fs::PermissionsExt;

use explorer_server::storage::{Volume, Volumes};

fn names(volumes: &Volumes) -> Vec<&str> {
    volumes.iter().map(Volume::name).collect()
}

#[test]
fn discovers_directories_sorted() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("media")).unwrap();
    fs::create_dir(dir.path().join("documents")).unwrap();
    fs::write(dir.path().join("notes.txt"), b"").unwrap();
    std::os::unix::fs::symlink("media", dir.path().join("link")).unwrap();

    let volumes = Volumes::discover(dir.path()).unwrap();

    assert_eq!(names(&volumes), ["documents", "media"]);
    assert_eq!(volumes.get("media").unwrap().name(), "media");
    assert!(volumes.get("link").is_none());
    assert!(volumes.get("notes.txt").is_none());
}

#[test]
fn read_only_volume_is_detected() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("documents");
    fs::create_dir(&locked).unwrap();
    fs::create_dir(dir.path().join("media")).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

    let volumes = Volumes::discover(dir.path());

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let volumes = volumes.unwrap();
    assert!(volumes.get("documents").unwrap().read_only());
    assert!(!volumes.get("media").unwrap().read_only());
}

#[test]
fn empty_data_dir_is_ok() {
    let dir = tempfile::tempdir().unwrap();

    let volumes = Volumes::discover(dir.path()).unwrap();

    assert_eq!(volumes.iter().count(), 0);
}

#[test]
fn missing_data_dir_is_error() {
    let dir = tempfile::tempdir().unwrap();

    assert!(Volumes::discover(&dir.path().join("missing")).is_err());
}
