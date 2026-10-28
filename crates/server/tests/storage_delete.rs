use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use explorer_server::storage::{RelPath, STAGING_DIR, StorageError, Volume, Volumes};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    volumes: Volumes,
}

impl Fixture {
    fn new(build: impl FnOnce(&Path, &Path)) -> Self {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir(data.path().join("media")).unwrap();
        fs::create_dir(data.path().join("outside")).unwrap();
        build(&data.path().join("media"), &data.path().join("outside"));
        // Only `media` is used; `outside` stands for anything beyond the volume.
        let volumes = Volumes::discover(data.path()).unwrap();
        Self { data, volumes }
    }

    fn media(&self) -> &Volume {
        self.volumes.get("media").unwrap()
    }

    fn media_path(&self) -> PathBuf {
        self.data.path().join("media")
    }

    fn outside_path(&self) -> PathBuf {
        self.data.path().join("outside")
    }
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

fn exists_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[test]
fn delete_file() {
    let fixture = Fixture::new(|media, _| {
        fs::create_dir(media.join("Films")).unwrap();
        fs::write(media.join("Films/a.txt"), b"a").unwrap();
        fs::write(media.join("Films/b.txt"), b"b").unwrap();
    });

    fixture.media().delete(&rel("Films/a.txt")).unwrap();

    assert!(!exists_no_follow(&fixture.media_path().join("Films/a.txt")));
    assert!(fixture.media_path().join("Films/b.txt").is_file());
}

#[test]
fn delete_folder_tree() {
    let fixture = Fixture::new(|media, _| {
        fs::create_dir_all(media.join("Films/2026/deep")).unwrap();
        fs::write(media.join("Films/a.txt"), b"a").unwrap();
        fs::write(media.join("Films/2026/deep/b.txt"), b"b").unwrap();
        fs::create_dir(media.join("Kept")).unwrap();
    });

    fixture.media().delete(&rel("Films")).unwrap();

    assert!(!exists_no_follow(&fixture.media_path().join("Films")));
    assert!(fixture.media_path().join("Kept").is_dir());
}

#[test]
fn delete_removes_symlink_not_target() {
    let fixture = Fixture::new(|media, outside| {
        fs::create_dir(media.join("real")).unwrap();
        fs::write(media.join("real/inner.txt"), b"inner").unwrap();
        fs::write(media.join("file.txt"), b"file").unwrap();
        fs::write(outside.join("secret.txt"), b"secret").unwrap();
        symlink("real", media.join("dirlink")).unwrap();
        symlink("file.txt", media.join("filelink")).unwrap();
        symlink(outside, media.join("outlink")).unwrap();
        // A tree holding a link to a folder: removing the tree must not descend into the target.
        fs::create_dir(media.join("tree")).unwrap();
        symlink("../real", media.join("tree/inlink")).unwrap();
    });
    let media = fixture.media();

    for link in ["dirlink", "filelink", "outlink", "tree"] {
        media.delete(&rel(link)).unwrap();
        assert!(
            !exists_no_follow(&fixture.media_path().join(link)),
            "{link}"
        );
    }

    assert_eq!(
        fs::read(fixture.media_path().join("real/inner.txt")).unwrap(),
        b"inner"
    );
    assert_eq!(
        fs::read(fixture.media_path().join("file.txt")).unwrap(),
        b"file"
    );
    assert_eq!(
        fs::read(fixture.outside_path().join("secret.txt")).unwrap(),
        b"secret"
    );
}

#[test]
fn delete_of_a_link_to_the_root_removes_only_the_link() {
    let fixture = Fixture::new(|media, _| {
        fs::write(media.join("a.txt"), b"a").unwrap();
        fs::create_dir(media.join("Sub")).unwrap();
        symlink(".", media.join("rootlink")).unwrap();
        symlink("..", media.join("Sub/up")).unwrap();
    });
    let media = fixture.media();

    for link in ["rootlink", "Sub/up"] {
        media.check_deletable(&rel(link)).unwrap();
        media.delete(&rel(link)).unwrap();
        assert!(
            !exists_no_follow(&fixture.media_path().join(link)),
            "{link}"
        );
    }
    assert_eq!(fs::read(fixture.media_path().join("a.txt")).unwrap(), b"a");
    assert!(fixture.media_path().join("Sub").is_dir());
}

#[test]
fn delete_refuses_root_and_reserved() {
    let fixture = Fixture::new(|media, _| {
        fs::create_dir(media.join(STAGING_DIR)).unwrap();
        fs::write(media.join(STAGING_DIR).join("x"), b"staged").unwrap();
        symlink(".", media.join("rootlink")).unwrap();
    });
    let media = fixture.media();

    assert!(matches!(
        media.delete(&RelPath::root()),
        Err(StorageError::InvalidPath(_))
    ));
    // The staging directory is no `RelPath`; through a link to the root it is not found.
    for reserved in [STAGING_DIR.to_owned(), format!("{STAGING_DIR}/x")] {
        assert!(RelPath::parse(&reserved).is_err(), "{reserved}");
    }
    for reserved in [
        format!("rootlink/{STAGING_DIR}"),
        format!("rootlink/{STAGING_DIR}/x"),
    ] {
        assert!(
            matches!(
                media.delete(&rel(&reserved)),
                Err(StorageError::NotFound(_))
            ),
            "{reserved}"
        );
    }

    assert_eq!(
        fs::read(fixture.media_path().join(STAGING_DIR).join("x")).unwrap(),
        b"staged"
    );
}

#[test]
fn delete_missing_is_not_found() {
    let fixture = Fixture::new(|media, _| {
        fs::create_dir(media.join("Films")).unwrap();
    });
    let media = fixture.media();

    for missing in ["nope", "Films/nope", "nope/deeper"] {
        assert!(
            matches!(media.delete(&rel(missing)), Err(StorageError::NotFound(_))),
            "{missing}"
        );
    }
}

#[test]
fn check_deletable_matches_delete() {
    let fixture = Fixture::new(|media, _| {
        fs::write(media.join("a.txt"), b"a").unwrap();
        symlink("missing", media.join("dangling")).unwrap();
        fs::create_dir(media.join(STAGING_DIR)).unwrap();
        symlink(".", media.join("rootlink")).unwrap();
    });
    let media = fixture.media();

    media.check_deletable(&rel("a.txt")).unwrap();
    media.check_deletable(&rel("dangling")).unwrap();
    assert!(matches!(
        media.check_deletable(&rel("nope")),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        media.check_deletable(&RelPath::root()),
        Err(StorageError::InvalidPath(_))
    ));
    assert!(matches!(
        media.check_deletable(&rel(&format!("rootlink/{STAGING_DIR}"))),
        Err(StorageError::NotFound(_))
    ));

    media.delete(&rel("dangling")).unwrap();
    assert!(!exists_no_follow(&fixture.media_path().join("dangling")));
}
