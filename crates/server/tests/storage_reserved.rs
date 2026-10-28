//! The staging directory stays out of reach whatever names lead to it.

use std::fs;

use explorer_server::storage::{RelPath, StorageError, Volumes};
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
}
