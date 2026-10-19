use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use explorer_server::storage::{
    EntryKind, EntryMeta, RelPath, StorageError, Volume, Volumes, WalkItem,
};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    volumes: Volumes,
}

impl Fixture {
    fn new(build: impl FnOnce(&Path)) -> Self {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir(data.path().join("media")).unwrap();
        fs::create_dir(data.path().join("documents")).unwrap();
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

fn find<'a>(entries: &'a [EntryMeta], name: &str) -> &'a EntryMeta {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}

fn assert_other(entry: &EntryMeta) {
    assert_eq!(entry.kind, EntryKind::Other, "{}", entry.name);
    assert_eq!(entry.size, None, "{}", entry.name);
}

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn lists_files_and_dirs_with_metadata() {
    let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let fixture = Fixture::new(|media| {
        write(&media.join("b.txt"), b"hello");
        fs::create_dir(media.join("a")).unwrap();
        fs::File::options()
            .write(true)
            .open(media.join("b.txt"))
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    });

    let entries = fixture.media().list(&RelPath::root()).unwrap();

    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a", "b.txt"]);
    assert_eq!(entries[0].kind, EntryKind::Dir);
    assert_eq!(entries[1].kind, EntryKind::File);
    assert_eq!(entries[1].size, Some(5));
    assert_eq!(entries[1].modified, Some(mtime));
}

#[test]
fn symlink_inside_volume_is_followed() {
    let fixture = Fixture::new(|media| {
        write(&media.join("photos/2026/a.jpg"), b"jpeg-bytes");
        symlink("2026/a.jpg", media.join("photos/latest")).unwrap();
    });

    let entries = fixture.media().list(&rel("photos")).unwrap();
    let latest = find(&entries, "latest");
    assert_eq!(latest.kind, EntryKind::File);
    assert_eq!(latest.size, Some(10));

    let stat = fixture.media().stat(&rel("photos/latest")).unwrap();
    assert_eq!(stat.kind, EntryKind::File);
    assert_eq!(stat.name, "latest");

    let (mut file, meta) = fixture.media().open_file(&rel("photos/latest")).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "jpeg-bytes");
    assert_eq!(meta.size, Some(10));
}

#[test]
fn symlink_outside_volume_is_other_and_unreadable() {
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("secret.txt"), b"secret");
    fs::create_dir(outside.path().join("x")).unwrap();
    let fixture = Fixture::new(|media| {
        symlink(outside.path().join("secret.txt"), media.join("escape")).unwrap();
        symlink(outside.path(), media.join("escape_dir")).unwrap();
    });
    let media = fixture.media();

    let entries = media.list(&RelPath::root()).unwrap();
    assert_other(find(&entries, "escape"));
    assert_other(find(&entries, "escape_dir"));

    for path in ["escape", "escape_dir"] {
        assert!(
            matches!(media.stat(&rel(path)), Err(StorageError::OutsideVolume(p)) if p == rel(path)),
            "stat {path}"
        );
    }
    assert!(matches!(
        media.open_file(&rel("escape")),
        Err(StorageError::OutsideVolume(_))
    ));
    assert!(matches!(
        media.list(&rel("escape_dir")),
        Err(StorageError::OutsideVolume(_))
    ));
    assert!(matches!(
        media.list(&rel("escape_dir/x")),
        Err(StorageError::OutsideVolume(_) | StorageError::NotFound(_))
    ));
    assert!(matches!(
        media.open_file(&rel("escape_dir/secret.txt")),
        Err(StorageError::OutsideVolume(_) | StorageError::NotFound(_))
    ));
    assert!(matches!(
        media.walk(&rel("escape_dir")),
        Err(StorageError::OutsideVolume(_))
    ));
}

#[test]
fn symlink_to_sibling_volume_is_outside() {
    let fixture = Fixture::new(|media| {
        symlink("../documents", media.join("docs")).unwrap();
    });
    write(&fixture.data.path().join("documents/report.txt"), b"report");
    let media = fixture.media();

    assert_other(find(&media.list(&RelPath::root()).unwrap(), "docs"));
    assert!(matches!(
        media.stat(&rel("docs")),
        Err(StorageError::OutsideVolume(_))
    ));
    assert!(matches!(
        media.list(&rel("docs")),
        Err(StorageError::OutsideVolume(_))
    ));
    assert!(matches!(
        media.open_file(&rel("docs/report.txt")),
        Err(StorageError::OutsideVolume(_))
    ));
}

#[test]
fn broken_symlink_is_other() {
    let fixture = Fixture::new(|media| {
        symlink("missing.txt", media.join("broken")).unwrap();
    });

    assert_other(find(
        &fixture.media().list(&RelPath::root()).unwrap(),
        "broken",
    ));
    assert!(matches!(
        fixture.media().stat(&rel("broken")),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn unicode_names_round_trip() {
    let fixture = Fixture::new(|media| {
        write(&media.join("é/😀 #?%.txt"), b"smile");
    });
    let media = fixture.media();

    let entries = media.list(&rel("é")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "😀 #?%.txt");

    let path = rel("é/😀 #?%.txt");
    assert_eq!(media.stat(&path).unwrap().size, Some(5));
    let (mut file, _) = media.open_file(&path).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "smile");
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_name_is_other() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fixture = Fixture::new(|media| {
        write(&media.join(OsStr::from_bytes(b"bad\xff")), b"x");
    });

    let entries = fixture.media().list(&RelPath::root()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "bad\u{fffd}");
    assert_other(&entries[0]);
}

#[test]
fn open_file_on_dir_is_is_a_directory() {
    let fixture = Fixture::new(|media| fs::create_dir(media.join("a")).unwrap());

    assert!(matches!(
        fixture.media().open_file(&rel("a")),
        Err(StorageError::IsADirectory(p)) if p == rel("a")
    ));
}

#[test]
fn list_on_file_is_not_a_directory() {
    let fixture = Fixture::new(|media| write(&media.join("b.txt"), b""));

    assert!(matches!(
        fixture.media().list(&rel("b.txt")),
        Err(StorageError::NotADirectory(p)) if p == rel("b.txt")
    ));
}

#[test]
fn missing_is_not_found() {
    let fixture = Fixture::new(|_| {});
    let media = fixture.media();
    let missing = rel("missing");

    assert!(matches!(media.list(&missing), Err(StorageError::NotFound(p)) if p == missing));
    assert!(matches!(media.stat(&missing), Err(StorageError::NotFound(p)) if p == missing));
    assert!(matches!(media.open_file(&missing), Err(StorageError::NotFound(p)) if p == missing));
    assert!(matches!(media.walk(&missing), Err(StorageError::NotFound(p)) if p == missing));
}

#[test]
fn unreadable_dir_is_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = Fixture::new(|media| {
        write(&media.join("locked/inner.txt"), b"x");
        write(&media.join("tree/locked/inner.txt"), b"x");
    });
    let set_mode = |path: &str, mode| {
        fs::set_permissions(
            fixture.media_path().join(path),
            fs::Permissions::from_mode(mode),
        )
        .unwrap();
    };
    set_mode("locked", 0o000);
    set_mode("tree/locked", 0o000);

    let list = fixture.media().list(&rel("locked"));
    let walk = fixture.media().walk(&rel("tree"));

    set_mode("locked", 0o755);
    set_mode("tree/locked", 0o755);
    assert!(matches!(list, Err(StorageError::PermissionDenied(p)) if p == rel("locked")));
    assert!(
        matches!(walk, Err(StorageError::PermissionDenied(ref p)) if *p == rel("tree/locked")),
        "{walk:?}"
    );
}

#[test]
fn walk_yields_dirs_then_files_and_skips_escapes() {
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("secret.txt"), b"secret");
    let fixture = Fixture::new(|media| {
        fs::create_dir_all(media.join("album/empty")).unwrap();
        write(&media.join("album/sub/song.mp3"), b"music");
        write(&media.join("album/cover.jpg"), b"jpg");
        symlink(outside.path(), media.join("album/escape_dir")).unwrap();
        symlink(
            outside.path().join("secret.txt"),
            media.join("album/escape"),
        )
        .unwrap();
        symlink("..", media.join("album/sub/loop")).unwrap();
    });

    let items = fixture.media().walk(&rel("album")).unwrap();

    let summary: Vec<_> = items
        .iter()
        .map(|item| match item {
            WalkItem::Dir(path) => format!("dir {}", path.as_str()),
            WalkItem::File { path, size, .. } => format!("file {} {size}", path.as_str()),
        })
        .collect();
    assert_eq!(
        summary,
        [
            "file album/cover.jpg 3",
            "dir album/empty",
            "dir album/sub",
            "file album/sub/song.mp3 5",
        ]
    );
}

#[test]
fn walk_root_covers_the_volume() {
    let fixture = Fixture::new(|media| write(&media.join("a/b.txt"), b"b"));

    let items = fixture.media().walk(&RelPath::root()).unwrap();

    assert!(
        matches!(&items[..], [WalkItem::Dir(a), WalkItem::File { path, .. }]
        if *a == rel("a") && *path == rel("a/b.txt"))
    );
}

#[test]
fn space_reports_nonzero_total() {
    let fixture = Fixture::new(|_| {});

    let space = fixture.media().space().unwrap();

    assert!(space.total > 0);
    assert!(space.available <= space.total);
}
