use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use explorer_server::storage::{
    RelPath, STAGING_DIR, StagedId, Staging, StorageError, Volume, Volumes, WalkItem,
};
use shared::api::conflict::ConflictPolicy;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    volumes: Volumes,
}

impl Fixture {
    fn new(build: impl FnOnce(&Path)) -> Self {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir(data.path().join("media")).unwrap();
        build(&data.path().join("media"));
        let volumes = Volumes::discover(data.path()).unwrap();
        Self { data, volumes }
    }

    fn media(&self) -> &Volume {
        self.volumes.get("media").unwrap()
    }

    fn media_path(&self) -> PathBuf {
        self.data.path().join("media")
    }
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn stage(staging: &Staging, contents: &[u8]) -> StagedId {
    let id = StagedId::random();
    let mut file = staging.create(&id).unwrap();
    file.write_all(contents).unwrap();
    id
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn create_dir_all_nested_and_idempotent() {
    let fixture = Fixture::new(|media| {
        fs::create_dir_all(media.join("real/inner")).unwrap();
        symlink("real", media.join("link")).unwrap();
    });
    let media = fixture.media();

    media.create_dir_all(&rel("a/b/c")).unwrap();
    media.create_dir_all(&rel("a/b/c")).unwrap();
    media.create_dir_all(&rel("a/b")).unwrap();
    media.create_dir_all(&RelPath::root()).unwrap();
    media.create_dir_all(&rel("link/inner/new")).unwrap();

    assert!(fixture.media_path().join("a/b/c").is_dir());
    assert!(fixture.media_path().join("real/inner/new").is_dir());
}

#[test]
fn create_dir_all_through_file_is_not_a_directory() {
    let fixture = Fixture::new(|media| write(&media.join("a/file"), b"x"));

    let result = fixture.media().create_dir_all(&rel("a/file/b/c"));

    assert!(
        matches!(result, Err(StorageError::NotADirectory(ref p)) if *p == rel("a/file")),
        "{result:?}"
    );
    assert!(!fixture.media_path().join("a/file/b").exists());
}

#[test]
fn create_dir_all_on_file_is_already_exists() {
    let fixture = Fixture::new(|media| write(&media.join("a/file"), b"x"));

    let result = fixture.media().create_dir_all(&rel("a/file"));

    assert!(
        matches!(result, Err(StorageError::AlreadyExists(ref p)) if *p == rel("a/file")),
        "{result:?}"
    );
}

#[test]
fn create_dir_all_through_escaping_symlink_is_outside_volume() {
    let outside = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(|media| symlink(outside.path(), media.join("escape")).unwrap());

    let result = fixture.media().create_dir_all(&rel("escape/x/y"));

    assert!(
        matches!(result, Err(StorageError::OutsideVolume(_))),
        "{result:?}"
    );
    assert!(names(outside.path()).is_empty());
}

#[test]
fn check_upload_target_by_policy() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a/file.txt"), b"x");
        fs::create_dir(media.join("a/folder")).unwrap();
    });
    let media = fixture.media();
    let check = |path: &str, policy| media.check_upload_target(&rel(path), policy);

    for policy in [
        ConflictPolicy::Fail,
        ConflictPolicy::Replace,
        ConflictPolicy::KeepBoth,
    ] {
        check("a/new.txt", policy).unwrap();
        check("missing/deep/new.txt", policy).unwrap();
        let through_file = check("a/file.txt/x", policy);
        assert!(
            matches!(through_file, Err(StorageError::NotADirectory(ref p)) if *p == rel("a/file.txt")),
            "{policy:?} {through_file:?}"
        );
    }

    for existing in ["a/file.txt", "a/folder"] {
        let result = check(existing, ConflictPolicy::Fail);
        assert!(
            matches!(result, Err(StorageError::AlreadyExists(ref p)) if *p == rel(existing)),
            "{result:?}"
        );
    }
    check("a/file.txt", ConflictPolicy::Replace).unwrap();
    let result = check("a/folder", ConflictPolicy::Replace);
    assert!(
        matches!(result, Err(StorageError::IsADirectory(ref p)) if *p == rel("a/folder")),
        "{result:?}"
    );
    check("a/file.txt", ConflictPolicy::KeepBoth).unwrap();
    check("a/folder", ConflictPolicy::KeepBoth).unwrap();
}

#[test]
fn commit_creates_folders_sets_mtime_and_empties_staging() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"movie");
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    let final_path = media
        .commit_staged(
            &staging,
            &id,
            &rel("Films/2026/Dune.mkv"),
            ConflictPolicy::Fail,
            Some(modified),
        )
        .unwrap();

    assert_eq!(final_path, rel("Films/2026/Dune.mkv"));
    let dest = fixture.media_path().join("Films/2026/Dune.mkv");
    assert_eq!(fs::read(&dest).unwrap(), b"movie");
    assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), modified);
    assert_eq!(staging.len(&id).unwrap(), None);
    assert!(staging.files().unwrap().is_empty());
    let mode = fs::metadata(fixture.media_path().join(STAGING_DIR))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn failed_commit_removes_the_folders_it_created() {
    let fixture = Fixture::new(|media| fs::create_dir(media.join("kept")).unwrap());
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let missing = StagedId::random();

    for dest in ["new/deep/a.txt", "kept/new/a.txt"] {
        let result =
            media.commit_staged(&staging, &missing, &rel(dest), ConflictPolicy::Fail, None);
        assert!(
            matches!(result, Err(StorageError::NotFound(ref p)) if *p == rel(dest)),
            "{result:?}"
        );
    }

    assert_eq!(names(&fixture.media_path()), [STAGING_DIR, "kept"]);
    assert!(names(&fixture.media_path().join("kept")).is_empty());
}

#[test]
fn dangling_symlink_on_the_way_is_refused_before_upload() {
    let fixture = Fixture::new(|media| symlink("missing", media.join("gone")).unwrap());
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"x");

    for policy in [
        ConflictPolicy::Fail,
        ConflictPolicy::Replace,
        ConflictPolicy::KeepBoth,
    ] {
        let check = media.check_upload_target(&rel("gone/sub/a.txt"), policy);
        assert!(
            matches!(check, Err(StorageError::NotADirectory(ref p)) if *p == rel("gone")),
            "{policy:?} {check:?}"
        );
    }
    let result = media.commit_staged(
        &staging,
        &id,
        &rel("gone/sub/a.txt"),
        ConflictPolicy::Fail,
        None,
    );
    assert!(
        matches!(result, Err(StorageError::NotADirectory(ref p)) if *p == rel("gone")),
        "{result:?}"
    );
    assert!(!fixture.media_path().join("missing").exists());
    assert_eq!(staging.len(&id).unwrap(), Some(1));
}

#[test]
fn commit_refuses_a_linked_staged_file() {
    let fixture = Fixture::new(|media| write(&media.join("target.txt"), b"target"));
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let staging_path = fixture.media_path().join(STAGING_DIR);

    let hard = stage(&staging, b"staged");
    fs::hard_link(
        staging_path.join(hard.as_str()),
        fixture.media_path().join("other"),
    )
    .unwrap();
    let symbolic = StagedId::random();
    symlink(
        fixture.media_path().join("target.txt"),
        staging_path.join(symbolic.as_str()),
    )
    .unwrap();

    for (id, dest) in [(&hard, "a.txt"), (&symbolic, "b.txt")] {
        let result = media.commit_staged(&staging, id, &rel(dest), ConflictPolicy::Replace, None);
        assert!(result.is_err(), "{dest}: {result:?}");
        assert!(!fixture.media_path().join(dest).exists(), "{dest}");
        assert!(staging.open_append(id).is_err(), "{dest}");
    }
    assert_eq!(
        fs::read(fixture.media_path().join("target.txt")).unwrap(),
        b"target"
    );
}

#[test]
fn staging_dir_is_created_exclusively() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| media.staging().map(drop)))
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
    });
    let mode = fs::symlink_metadata(fixture.media_path().join(STAGING_DIR))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    let outside = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(|media| {
        symlink(outside.path().join("made"), media.join(STAGING_DIR)).unwrap();
    });
    assert!(fixture.media().staging().is_err());
    assert!(names(outside.path()).is_empty());
}

#[test]
fn staging_tracks_files_and_appends() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    assert!(media.existing_staging().unwrap().is_none());
    let staging = media.staging().unwrap();
    assert!(media.existing_staging().unwrap().is_some());
    let id = stage(&staging, b"abc");

    assert!(matches!(
        staging.create(&id),
        Err(StorageError::AlreadyExists(_))
    ));
    let (mut file, len) = staging.open_append(&id).unwrap();
    assert_eq!(len, 3);
    file.write_all(b"def").unwrap();
    assert_eq!(staging.len(&id).unwrap(), Some(6));
    let path = fixture.media_path().join(STAGING_DIR).join(id.as_str());
    assert_eq!(fs::read(&path).unwrap(), b"abcdef");
    let files = staging.files().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, id.as_str());

    staging.remove(&id).unwrap();
    staging.remove(&id).unwrap();
    assert_eq!(staging.len(&id).unwrap(), None);
    assert!(matches!(
        staging.open_append(&id),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn commit_keep_both_picks_next_free_name() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.txt"), b"first");
        write(&media.join("a (1).txt"), b"second");
    });
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"third");

    let final_path = media
        .commit_staged(&staging, &id, &rel("a.txt"), ConflictPolicy::KeepBoth, None)
        .unwrap();

    assert_eq!(final_path, rel("a (2).txt"));
    let root = fixture.media_path();
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"first");
    assert_eq!(fs::read(root.join("a (1).txt")).unwrap(), b"second");
    assert_eq!(fs::read(root.join("a (2).txt")).unwrap(), b"third");
}

#[test]
fn commit_replace_overwrites_file() {
    let fixture = Fixture::new(|media| write(&media.join("dir/a.txt"), b"old"));
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"new");

    let final_path = media
        .commit_staged(
            &staging,
            &id,
            &rel("dir/a.txt"),
            ConflictPolicy::Replace,
            None,
        )
        .unwrap();

    assert_eq!(final_path, rel("dir/a.txt"));
    assert_eq!(
        fs::read(fixture.media_path().join("dir/a.txt")).unwrap(),
        b"new"
    );
    assert_eq!(names(&fixture.media_path().join("dir")), ["a.txt"]);
    assert_eq!(staging.len(&id).unwrap(), None);
}

#[test]
fn commit_replace_refuses_folder() {
    let fixture = Fixture::new(|media| write(&media.join("folder/inner.txt"), b"keep"));
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"new");

    let result = media.commit_staged(&staging, &id, &rel("folder"), ConflictPolicy::Replace, None);

    assert!(
        matches!(result, Err(StorageError::IsADirectory(ref p)) if *p == rel("folder")),
        "{result:?}"
    );
    assert_eq!(
        fs::read(fixture.media_path().join("folder/inner.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(staging.len(&id).unwrap(), Some(3));
}

#[test]
fn commit_fail_after_race_keeps_both() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"upload");
    media
        .check_upload_target(&rel("a.txt"), ConflictPolicy::Fail)
        .unwrap();
    write(&fixture.media_path().join("a.txt"), b"original");

    let final_path = media
        .commit_staged(&staging, &id, &rel("a.txt"), ConflictPolicy::Fail, None)
        .unwrap();

    assert_eq!(final_path, rel("a (1).txt"));
    let root = fixture.media_path();
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"original");
    assert_eq!(fs::read(root.join("a (1).txt")).unwrap(), b"upload");
}

#[test]
fn staging_dir_is_hidden_and_reserved() {
    let fixture = Fixture::new(|media| write(&media.join("a.txt"), b"x"));
    let media = fixture.media();
    let staging = media.staging().unwrap();
    stage(&staging, b"pending");
    assert!(fixture.media_path().join(STAGING_DIR).is_dir());

    let listed: Vec<_> = media
        .list(&RelPath::root())
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(listed, ["a.txt"]);
    let walked: Vec<_> = media
        .walk(&RelPath::root())
        .unwrap()
        .into_iter()
        .map(|item| match item {
            WalkItem::Dir(path) | WalkItem::File { path, .. } => path,
        })
        .collect();
    assert_eq!(walked, [rel("a.txt")]);
    assert!(RelPath::parse(STAGING_DIR).is_err());
}

#[test]
fn staging_symlink_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(|media| symlink(outside.path(), media.join(STAGING_DIR)).unwrap());
    let media = fixture.media();

    assert!(media.staging().is_err());
    assert!(media.existing_staging().is_err());
    assert!(names(outside.path()).is_empty());
}

#[test]
fn staging_non_directory_is_refused() {
    let fixture = Fixture::new(|media| write(&media.join(STAGING_DIR), b"x"));

    assert!(fixture.media().staging().is_err());
}

#[test]
fn commit_into_escaping_parent_is_outside_volume() {
    let outside = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(|media| symlink(outside.path(), media.join("escape")).unwrap());
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"payload");

    for policy in [
        ConflictPolicy::Fail,
        ConflictPolicy::Replace,
        ConflictPolicy::KeepBoth,
    ] {
        let check = media.check_upload_target(&rel("escape/x"), policy);
        assert!(
            matches!(check, Err(StorageError::OutsideVolume(_))),
            "{check:?}"
        );
        let result = media.commit_staged(&staging, &id, &rel("escape/x"), policy, None);
        assert!(
            matches!(result, Err(StorageError::OutsideVolume(_))),
            "{policy:?} {result:?}"
        );
    }
    assert!(names(outside.path()).is_empty());
    assert_eq!(staging.len(&id).unwrap(), Some(7));
}

#[test]
fn commit_to_root_is_a_directory() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"x");

    let result = media.commit_staged(
        &staging,
        &id,
        &RelPath::root(),
        ConflictPolicy::KeepBoth,
        None,
    );

    assert!(
        matches!(result, Err(StorageError::IsADirectory(ref p)) if p.is_root()),
        "{result:?}"
    );
}

#[test]
fn committed_file_mode_follows_the_umask() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"x");
    media
        .commit_staged(&staging, &id, &rel("a.txt"), ConflictPolicy::Fail, None)
        .unwrap();

    let reference = fixture.media_path().join("reference");
    fs::File::create(&reference).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fixture.media_path().join("a.txt")), mode(&reference));
}

#[test]
fn staging_is_unreachable_through_a_symlink_to_the_root() {
    let fixture = Fixture::new(|media| {
        symlink(".", media.join("self")).unwrap();
        symlink(STAGING_DIR, media.join("staged")).unwrap();
        write(&media.join("a.txt"), b"a");
    });
    let media = fixture.media();
    let staging = media.staging().unwrap();
    let id = stage(&staging, b"pending");
    let staged = rel(&format!("self/{STAGING_DIR}/{}", id.as_str()));
    let not_found = |result: Result<(), StorageError>| {
        assert!(
            matches!(result, Err(StorageError::NotFound(_))),
            "{result:?}"
        );
    };

    let names = |dir: &str| -> Vec<String> {
        media
            .list(&rel(dir))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    };
    assert_eq!(names("self"), ["a.txt", "self"]);
    assert_eq!(names("self/self"), ["a.txt", "self"]);
    assert_eq!(names(""), ["a.txt", "self"]);
    for dir in ["self/.explorer-uploads", "staged", "self/staged"] {
        not_found(media.list(&rel(dir)).map(drop));
    }
    let walked: Vec<_> = media
        .walk(&rel("self"))
        .unwrap()
        .into_iter()
        .map(|item| match item {
            WalkItem::Dir(path) | WalkItem::File { path, .. } => path,
        })
        .collect();
    assert_eq!(walked, [rel("self/a.txt")]);
    not_found(media.walk(&rel("staged")).map(drop));

    not_found(media.stat(&staged).map(drop));
    not_found(media.open_file(&staged).map(drop));
    not_found(
        media
            .stat(&rel(&format!("staged/{}", id.as_str())))
            .map(drop),
    );
    not_found(media.check_upload_target(&staged, ConflictPolicy::Replace));
    not_found(media.check_upload_target(&rel("self/.explorer-uploads/x/y"), ConflictPolicy::Fail));
    not_found(media.create_dir_all(&rel("self/.explorer-uploads/sub")));
    let other = stage(&staging, b"other");
    not_found(
        media
            .commit_staged(&staging, &other, &staged, ConflictPolicy::Replace, None)
            .map(drop),
    );
    assert_eq!(
        fs::read(fixture.media_path().join(STAGING_DIR).join(id.as_str())).unwrap(),
        b"pending"
    );
    assert_eq!(names(""), ["a.txt", "self"]);
}

#[test]
fn rename_file_and_folder() {
    let fixture = Fixture::new(|media| {
        write(&media.join("dir/a.txt"), b"a");
        write(&media.join("Films/inner.txt"), b"inner");
    });
    let media = fixture.media();

    let renamed = media
        .rename(&rel("dir/a.txt"), "b.txt", ConflictPolicy::Fail)
        .unwrap();
    assert_eq!(renamed, rel("dir/b.txt"));
    let renamed = media
        .rename(&rel("Films"), "Movies", ConflictPolicy::Fail)
        .unwrap();
    assert_eq!(renamed, rel("Movies"));

    let root = fixture.media_path();
    assert_eq!(names(&root.join("dir")), ["b.txt"]);
    assert_eq!(fs::read(root.join("Movies/inner.txt")).unwrap(), b"inner");
    assert!(!root.join("Films").exists());
}

#[test]
fn rename_refuses_existing_under_fail() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.txt"), b"a");
        write(&media.join("b.txt"), b"b");
        fs::create_dir(media.join("folder")).unwrap();
    });
    let media = fixture.media();

    for target in ["b.txt", "folder"] {
        let result = media.rename(&rel("a.txt"), target, ConflictPolicy::Fail);
        assert!(
            matches!(result, Err(StorageError::AlreadyExists(ref p)) if *p == rel(target)),
            "{result:?}"
        );
    }
    assert_eq!(names(&fixture.media_path()), ["a.txt", "b.txt", "folder"]);
}

#[test]
fn rename_keep_both_suffixes() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.txt"), b"a");
        write(&media.join("b.txt"), b"b");
        write(&media.join("b (1).txt"), b"b1");
        fs::create_dir(media.join("x")).unwrap();
        fs::create_dir(media.join("Films")).unwrap();
    });
    let media = fixture.media();

    let renamed = media
        .rename(&rel("a.txt"), "b.txt", ConflictPolicy::KeepBoth)
        .unwrap();
    assert_eq!(renamed, rel("b (2).txt"));
    let renamed = media
        .rename(&rel("x"), "Films", ConflictPolicy::KeepBoth)
        .unwrap();
    assert_eq!(renamed, rel("Films (1)"));

    let root = fixture.media_path();
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"b");
    assert_eq!(fs::read(root.join("b (2).txt")).unwrap(), b"a");
    assert_eq!(
        names(&root),
        ["Films", "Films (1)", "b (1).txt", "b (2).txt", "b.txt"]
    );
}

#[test]
fn rename_replace_file_over_file() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.txt"), b"new");
        write(&media.join("b.txt"), b"old");
    });
    let media = fixture.media();

    let renamed = media
        .rename(&rel("a.txt"), "b.txt", ConflictPolicy::Replace)
        .unwrap();

    assert_eq!(renamed, rel("b.txt"));
    let root = fixture.media_path();
    assert_eq!(names(&root), ["b.txt"]);
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"new");
}

#[test]
fn rename_replace_refuses_folders() {
    let fixture = Fixture::new(|media| {
        write(&media.join("file.txt"), b"file");
        write(&media.join("other.txt"), b"other");
        write(&media.join("folder/inner.txt"), b"inner");
        fs::create_dir(media.join("empty")).unwrap();
    });
    let media = fixture.media();

    for (from, to) in [
        ("file.txt", "folder"),
        ("folder", "other.txt"),
        ("empty", "folder"),
        ("folder", "empty"),
    ] {
        let result = media.rename(&rel(from), to, ConflictPolicy::Replace);
        assert!(
            matches!(result, Err(StorageError::IsADirectory(_))),
            "{from} -> {to}: {result:?}"
        );
    }
    let root = fixture.media_path();
    assert_eq!(names(&root), ["empty", "file.txt", "folder", "other.txt"]);
    assert_eq!(fs::read(root.join("folder/inner.txt")).unwrap(), b"inner");
}

#[test]
fn rename_symlink_renames_the_link() {
    let fixture = Fixture::new(|media| {
        write(&media.join("target.txt"), b"target");
        fs::create_dir(media.join("dir")).unwrap();
        symlink("target.txt", media.join("link")).unwrap();
        symlink("dir", media.join("dirlink")).unwrap();
    });
    let media = fixture.media();

    media
        .rename(&rel("link"), "renamed", ConflictPolicy::Fail)
        .unwrap();
    media
        .rename(&rel("dirlink"), "renamed-dir", ConflictPolicy::Fail)
        .unwrap();

    let root = fixture.media_path();
    assert_eq!(
        names(&root),
        ["dir", "renamed", "renamed-dir", "target.txt"]
    );
    assert_eq!(
        fs::read_link(root.join("renamed")).unwrap(),
        Path::new("target.txt")
    );
    assert_eq!(
        fs::read_link(root.join("renamed-dir")).unwrap(),
        Path::new("dir")
    );
    assert_eq!(fs::read(root.join("target.txt")).unwrap(), b"target");
    assert!(root.join("dir").is_dir());
}

#[test]
fn rename_refuses_root_and_reserved() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.txt"), b"a");
        symlink(".", media.join("root")).unwrap();
    });
    let media = fixture.media();
    media.staging().unwrap();

    assert!(RelPath::parse(".explorer-uploads/x").is_err());
    let result = media.rename(&RelPath::root(), "x", ConflictPolicy::Fail);
    assert!(
        matches!(result, Err(StorageError::InvalidPath(_))),
        "{result:?}"
    );
    for name in [STAGING_DIR, "a/b", "", ".."] {
        let result = media.rename(&rel("a.txt"), name, ConflictPolicy::Fail);
        assert!(
            matches!(result, Err(StorageError::InvalidPath(_))),
            "{name:?}: {result:?}"
        );
    }
    let result = media.rename(&rel("root/a.txt"), STAGING_DIR, ConflictPolicy::KeepBoth);
    assert!(
        matches!(result, Err(StorageError::NotFound(_))),
        "{result:?}"
    );
    let result = media.rename(&rel("root/.explorer-uploads"), "x", ConflictPolicy::Fail);
    assert!(
        matches!(result, Err(StorageError::NotFound(_))),
        "{result:?}"
    );

    let root = fixture.media_path();
    assert_eq!(names(&root), [STAGING_DIR, "a.txt", "root"]);
    assert!(root.join(STAGING_DIR).is_dir());
}

#[test]
fn rename_to_same_name_is_noop() {
    let fixture = Fixture::new(|media| write(&media.join("dir/a.txt"), b"a"));
    let media = fixture.media();

    for conflict in [
        ConflictPolicy::Fail,
        ConflictPolicy::Replace,
        ConflictPolicy::KeepBoth,
    ] {
        let renamed = media.rename(&rel("dir/a.txt"), "a.txt", conflict).unwrap();
        assert_eq!(renamed, rel("dir/a.txt"));
    }
    assert_eq!(names(&fixture.media_path().join("dir")), ["a.txt"]);
    let result = media.rename(&rel("dir/missing"), "missing", ConflictPolicy::Fail);
    assert!(
        matches!(result, Err(StorageError::NotFound(_))),
        "{result:?}"
    );
}

#[test]
fn rename_missing_source_is_not_found() {
    let fixture = Fixture::new(|_| {});
    let result = fixture
        .media()
        .rename(&rel("missing"), "other", ConflictPolicy::KeepBoth);
    assert!(
        matches!(result, Err(StorageError::NotFound(ref p)) if *p == rel("missing")),
        "{result:?}"
    );
}
