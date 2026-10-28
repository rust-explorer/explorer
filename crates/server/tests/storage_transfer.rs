use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use explorer_server::storage::{
    self, COPY_CHUNK, MoveOutcome, RelPath, STAGING_DIR, StorageError, TransferProgress, Volume,
    Volumes,
};
use shared::api::conflict::ConflictPolicy;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    volumes: Volumes,
}

impl Fixture {
    fn new(build: impl FnOnce(&Path, &Path)) -> Self {
        let data = tempfile::tempdir().unwrap();
        for name in ["media", "documents"] {
            fs::create_dir(data.path().join(name)).unwrap();
        }
        build(&data.path().join("media"), &data.path().join("documents"));
        let volumes = Volumes::discover(data.path()).unwrap();
        Self { data, volumes }
    }

    fn media(&self) -> &Volume {
        self.volumes.get("media").unwrap()
    }

    fn documents(&self) -> &Volume {
        self.volumes.get("documents").unwrap()
    }

    fn path(&self, volume: &str) -> PathBuf {
        self.data.path().join(volume)
    }

    fn staged(&self, volume: &str) -> Vec<String> {
        match fs::read_dir(self.path(volume).join(STAGING_DIR)) {
            Ok(entries) => entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Counts what the copy reports; cancels once `cancel_after_bytes` or `cancel_after_files` is
/// reached.
#[derive(Default)]
struct Recorder {
    bytes: AtomicU64,
    files: AtomicU64,
    skipped: AtomicU64,
    cancelled: AtomicBool,
    cancel_after_bytes: Option<u64>,
    cancel_after_files: Option<u64>,
}

impl TransferProgress for Recorder {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn add_bytes(&self, n: u64) {
        let total = self.bytes.fetch_add(n, Ordering::Relaxed) + n;
        if self.cancel_after_bytes.is_some_and(|limit| total >= limit) {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn add_file(&self) {
        let total = self.files.fetch_add(1, Ordering::Relaxed) + 1;
        if self.cancel_after_files.is_some_and(|limit| total >= limit) {
            self.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn mtime() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

/// Checks, plans and copies `path` of `source` into `dest_dir` of `dest`.
fn copy(
    source: &Volume,
    path: &str,
    dest: &Volume,
    dest_dir: &str,
    conflict: ConflictPolicy,
    progress: &Recorder,
) -> Result<RelPath, StorageError> {
    let (path, dest_dir) = (rel(path), rel(dest_dir));
    storage::check_transfer(source, &path, dest, &dest_dir, conflict)?;
    let plan = storage::plan_transfer(source, &path, progress)?;
    storage::copy(source, &path, dest, &dest_dir, conflict, &plan, progress)
}

#[test]
fn copy_file_same_volume() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"hello");
        fs::create_dir(media.join("Sub")).unwrap();
    });
    let progress = Recorder::default();

    let placed = copy(
        fixture.media(),
        "a.txt",
        fixture.media(),
        "Sub",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    assert_eq!(placed, rel("Sub/a.txt"));
    let media = fixture.path("media");
    assert_eq!(fs::read(media.join("Sub/a.txt")).unwrap(), b"hello");
    assert_eq!(fs::read(media.join("a.txt")).unwrap(), b"hello");
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 5);
    assert_eq!(progress.files.load(Ordering::Relaxed), 1);
    assert!(fixture.staged("media").is_empty());
}

#[test]
fn copy_file_across_volumes() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("Films/a.mkv"), b"film");
        fs::File::options()
            .write(true)
            .open(media.join("Films/a.mkv"))
            .unwrap()
            .set_modified(mtime())
            .unwrap();
    });
    let plan =
        storage::plan_transfer(fixture.media(), &rel("Films/a.mkv"), &Recorder::default()).unwrap();
    assert_eq!((plan.files, plan.bytes, plan.items.len()), (1, 4, 1));

    let placed = copy(
        fixture.media(),
        "Films/a.mkv",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &Recorder::default(),
    )
    .unwrap();

    assert_eq!(placed, rel("a.mkv"));
    let copied = fixture.path("documents").join("a.mkv");
    assert_eq!(fs::read(&copied).unwrap(), b"film");
    assert_eq!(fs::metadata(&copied).unwrap().modified().unwrap(), mtime());
    assert!(fixture.staged("documents").is_empty());
}

#[test]
fn copy_folder_tree() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/a.txt"), b"abc");
        write(&media.join("F/Deep/inner/b.txt"), b"defg");
        write(&media.join("F/Deep/zero.bin"), b"");
        fs::create_dir(media.join("F/Empty")).unwrap();
        fs::File::options()
            .write(true)
            .open(media.join("F/a.txt"))
            .unwrap()
            .set_modified(mtime())
            .unwrap();
    });
    let plan = storage::plan_transfer(fixture.media(), &rel("F"), &Recorder::default()).unwrap();
    assert_eq!((plan.files, plan.bytes, plan.skipped), (3, 7, 0));
    let progress = Recorder::default();

    let placed = copy(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    assert_eq!(placed, rel("F"));
    let copied = fixture.path("documents").join("F");
    assert_eq!(fs::read(copied.join("a.txt")).unwrap(), b"abc");
    assert_eq!(fs::read(copied.join("Deep/inner/b.txt")).unwrap(), b"defg");
    assert_eq!(fs::read(copied.join("Deep/zero.bin")).unwrap(), b"");
    assert!(copied.join("Empty").is_dir());
    assert_eq!(
        fs::metadata(copied.join("a.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime()
    );
    assert_eq!(progress.files.load(Ordering::Relaxed), 3);
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 7);
    assert!(fixture.path("media").join("F/Deep/inner/b.txt").is_file());
    assert!(fixture.staged("documents").is_empty());
}

#[test]
fn copy_keep_both_names_the_copy() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"a");
        write(&media.join("F/b.txt"), b"b");
    });
    let progress = Recorder::default();

    let file = copy(
        fixture.media(),
        "a.txt",
        fixture.media(),
        "",
        ConflictPolicy::KeepBoth,
        &progress,
    )
    .unwrap();
    let folder = copy(
        fixture.media(),
        "F",
        fixture.media(),
        "",
        ConflictPolicy::KeepBoth,
        &progress,
    )
    .unwrap();

    assert_eq!(file, rel("a (1).txt"));
    assert_eq!(folder, rel("F (1)"));
    let media = fixture.path("media");
    assert_eq!(fs::read(media.join("a (1).txt")).unwrap(), b"a");
    assert_eq!(fs::read(media.join("F (1)/b.txt")).unwrap(), b"b");
    assert_eq!(fs::read(media.join("F/b.txt")).unwrap(), b"b");
}

#[test]
fn copy_replace_file() {
    let fixture = Fixture::new(|media, documents| {
        write(&media.join("a.txt"), b"new");
        write(&documents.join("a.txt"), b"old");
    });

    let placed = copy(
        fixture.media(),
        "a.txt",
        fixture.documents(),
        "",
        ConflictPolicy::Replace,
        &Recorder::default(),
    )
    .unwrap();

    assert_eq!(placed, rel("a.txt"));
    assert_eq!(
        fs::read(fixture.path("documents").join("a.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn check_refuses_replace_with_folders() {
    let fixture = Fixture::new(|media, documents| {
        write(&media.join("a.txt"), b"a");
        write(&media.join("F/x"), b"x");
        write(&media.join("G/x"), b"x");
        fs::create_dir(documents.join("a.txt")).unwrap();
        fs::create_dir(documents.join("F")).unwrap();
        write(&documents.join("G"), b"file");
    });
    let check = |path: &str, conflict| {
        storage::check_transfer(
            fixture.media(),
            &rel(path),
            fixture.documents(),
            &RelPath::root(),
            conflict,
        )
    };

    for path in ["a.txt", "F", "G"] {
        assert!(
            matches!(check(path, ConflictPolicy::Replace), Err(StorageError::IsADirectory(target)) if target == rel(path)),
            "{path}"
        );
        assert!(
            matches!(check(path, ConflictPolicy::Fail), Err(StorageError::AlreadyExists(target)) if target == rel(path)),
            "{path}"
        );
        check(path, ConflictPolicy::KeepBoth).unwrap();
    }
}

#[test]
fn check_needs_an_existing_folder_and_a_source() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"a");
        write(&media.join("file"), b"f");
    });
    let check = |path: &str, dest_dir: &str| {
        storage::check_transfer(
            fixture.media(),
            &rel(path),
            fixture.documents(),
            &rel(dest_dir),
            ConflictPolicy::Fail,
        )
    };
    let check_media = |dest_dir: &str| {
        storage::check_transfer(
            fixture.media(),
            &rel("a.txt"),
            fixture.media(),
            &rel(dest_dir),
            ConflictPolicy::Fail,
        )
    };

    assert!(matches!(
        check("a.txt", "nope"),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        check_media("file"),
        Err(StorageError::NotADirectory(_))
    ));
    assert!(matches!(
        check("missing", ""),
        Err(StorageError::Source(inner)) if matches!(*inner, StorageError::NotFound(_))
    ));
    assert!(matches!(check("", ""), Err(StorageError::InvalidPath(_))));
    check("a.txt", "").unwrap();
}

#[test]
fn copy_into_itself_is_refused() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("A/sub/x.txt"), b"x");
        symlink("A", media.join("link")).unwrap();
        symlink(".", media.join("rootlink")).unwrap();
    });
    let check = |path: &str, dest_dir: &str| {
        storage::check_transfer(
            fixture.media(),
            &rel(path),
            fixture.media(),
            &rel(dest_dir),
            ConflictPolicy::KeepBoth,
        )
    };

    for dest_dir in ["A", "A/sub", "link", "link/sub"] {
        assert!(
            matches!(
                check("A", dest_dir),
                Err(StorageError::InvalidDestination(path)) if path == rel("A")
            ),
            "{dest_dir}"
        );
    }
    assert!(matches!(
        check("rootlink", "A"),
        Err(StorageError::InvalidPath(_))
    ));
    check("A", "").unwrap();
    check("A/sub/x.txt", "A/sub").unwrap();
}

#[test]
fn copy_skips_symlinked_dirs_and_fifos() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/a.txt"), b"a");
        write(&media.join("Other/b.txt"), b"b");
        symlink("../Other", media.join("F/dirlink")).unwrap();
        let fifo = media.join("F/fifo");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
    });
    let plan = storage::plan_transfer(fixture.media(), &rel("F"), &Recorder::default()).unwrap();
    assert_eq!((plan.files, plan.skipped), (1, 2));
    let progress = Recorder::default();

    copy(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    let copied = fixture.path("documents").join("F");
    let mut names: Vec<String> = fs::read_dir(&copied)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt"]);
    assert_eq!(progress.skipped.load(Ordering::Relaxed), 2);
}

#[test]
fn copy_follows_file_symlinks() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("real.txt"), b"real");
        fs::create_dir(media.join("F")).unwrap();
        symlink("../real.txt", media.join("F/link.txt")).unwrap();
        symlink("real.txt", media.join("top.txt")).unwrap();
    });

    copy(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &Recorder::default(),
    )
    .unwrap();
    copy(
        fixture.media(),
        "top.txt",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &Recorder::default(),
    )
    .unwrap();

    let documents = fixture.path("documents");
    for copied in [documents.join("F/link.txt"), documents.join("top.txt")] {
        let meta = fs::symlink_metadata(&copied).unwrap();
        assert!(meta.is_file(), "{}", copied.display());
        assert_eq!(fs::read(&copied).unwrap(), b"real");
    }
}

#[test]
fn target_appearing_after_check_is_kept() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"mine");
        write(&media.join("F/x.txt"), b"x");
    });
    for path in ["a.txt", "F"] {
        storage::check_transfer(
            fixture.media(),
            &rel(path),
            fixture.documents(),
            &RelPath::root(),
            ConflictPolicy::Fail,
        )
        .unwrap();
    }
    let documents = fixture.path("documents");
    write(&documents.join("a.txt"), b"theirs");
    write(&documents.join("F/theirs.txt"), b"theirs");

    for (path, expected) in [("a.txt", "a (1).txt"), ("F", "F (1)")] {
        let plan =
            storage::plan_transfer(fixture.media(), &rel(path), &Recorder::default()).unwrap();
        let placed = storage::copy(
            fixture.media(),
            &rel(path),
            fixture.documents(),
            &RelPath::root(),
            ConflictPolicy::Fail,
            &plan,
            &Recorder::default(),
        )
        .unwrap();
        assert_eq!(placed, rel(expected));
    }

    assert_eq!(fs::read(documents.join("a.txt")).unwrap(), b"theirs");
    assert_eq!(fs::read(documents.join("a (1).txt")).unwrap(), b"mine");
    assert!(documents.join("F/theirs.txt").is_file());
    assert!(!documents.join("F/x.txt").exists());
    assert_eq!(fs::read(documents.join("F (1)/x.txt")).unwrap(), b"x");
}

#[test]
fn cancelled_copy_leaves_nothing() {
    let big = vec![7u8; 3 * COPY_CHUNK];
    let fixture = Fixture::new(|media, _| {
        write(&media.join("big.bin"), &big);
        for name in ["a", "b", "c", "d"] {
            write(&media.join(format!("F/{name}/file.txt")), name.as_bytes());
        }
    });

    let progress = Recorder {
        cancel_after_bytes: Some(COPY_CHUNK as u64),
        ..Recorder::default()
    };
    let result = copy(
        fixture.media(),
        "big.bin",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    );
    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert_eq!(progress.bytes.load(Ordering::Relaxed), COPY_CHUNK as u64);

    let progress = Recorder {
        cancel_after_files: Some(2),
        ..Recorder::default()
    };
    let result = copy(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    );
    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert_eq!(progress.files.load(Ordering::Relaxed), 2);

    let documents = fixture.path("documents");
    assert!(!documents.join("big.bin").exists());
    assert!(!documents.join("F").exists());
    assert!(fixture.staged("documents").is_empty());
}

/// Checks and moves `path` of `source` into `dest_dir` of `dest`, planning only when needed.
fn move_entry(
    source: &Volume,
    path: &str,
    dest: &Volume,
    dest_dir: &str,
    conflict: ConflictPolicy,
    progress: &Recorder,
) -> Result<MoveOutcome, StorageError> {
    let (path, dest_dir) = (rel(path), rel(dest_dir));
    storage::check_move(source, &path, dest, &dest_dir, conflict)?;
    storage::move_entry(source, &path, dest, &dest_dir, conflict, None, progress)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn mkfifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn move_file_same_volume() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"hello");
        fs::create_dir(media.join("Sub")).unwrap();
    });
    let progress = Recorder::default();

    let outcome = move_entry(
        fixture.media(),
        "a.txt",
        fixture.media(),
        "Sub",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    assert!(
        matches!(outcome, MoveOutcome::Renamed { final_path } if final_path == rel("Sub/a.txt"))
    );
    let media = fixture.path("media");
    assert_eq!(fs::read(media.join("Sub/a.txt")).unwrap(), b"hello");
    assert!(!media.join("a.txt").exists());
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 0);
}

#[test]
fn move_folder_same_volume_keeps_symlinks() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/real.txt"), b"real");
        symlink("real.txt", media.join("F/link")).unwrap();
        fs::create_dir(media.join("Sub")).unwrap();
    });

    let outcome = move_entry(
        fixture.media(),
        "F",
        fixture.media(),
        "Sub",
        ConflictPolicy::Fail,
        &Recorder::default(),
    )
    .unwrap();

    assert!(matches!(outcome, MoveOutcome::Renamed { final_path } if final_path == rel("Sub/F")));
    let moved = fixture.path("media").join("Sub/F");
    assert!(
        fs::symlink_metadata(moved.join("link"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(moved.join("link")).unwrap(),
        Path::new("real.txt")
    );
    assert_eq!(fs::read(moved.join("link")).unwrap(), b"real");
    assert!(!fixture.path("media").join("F").exists());
}

#[test]
fn move_across_volumes() {
    let fixture = Fixture::new(|media, documents| {
        write(&media.join("a.txt"), b"abc");
        write(&media.join("F/x.txt"), b"x");
        write(&media.join("F/Deep/inner/y.txt"), b"yy");
        fs::create_dir(media.join("F/Empty")).unwrap();
        fs::create_dir(documents.join("Backup")).unwrap();
    });
    let progress = Recorder::default();

    let file = move_entry(
        fixture.media(),
        "a.txt",
        fixture.documents(),
        "Backup",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();
    let tree = move_entry(
        fixture.media(),
        "F",
        fixture.documents(),
        "Backup",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    assert!(
        matches!(file, MoveOutcome::Copied { final_path } if final_path == rel("Backup/a.txt"))
    );
    assert!(matches!(tree, MoveOutcome::Copied { final_path } if final_path == rel("Backup/F")));
    let backup = fixture.path("documents").join("Backup");
    assert_eq!(fs::read(backup.join("a.txt")).unwrap(), b"abc");
    assert_eq!(fs::read(backup.join("F/x.txt")).unwrap(), b"x");
    assert_eq!(fs::read(backup.join("F/Deep/inner/y.txt")).unwrap(), b"yy");
    assert!(backup.join("F/Empty").is_dir());
    assert_eq!(names(&fixture.path("media")), Vec::<String>::new());
    assert_eq!(progress.files.load(Ordering::Relaxed), 3);
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 6);
    assert!(fixture.staged("documents").is_empty());
}

#[test]
fn cross_volume_move_keeps_what_it_skipped() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/a.txt"), b"a");
        write(&media.join("F/Gone/b.txt"), b"b");
        write(&media.join("Other/c.txt"), b"c");
        fs::create_dir_all(media.join("F/Keep/Deep")).unwrap();
        symlink("../../Other", media.join("F/Keep/dirlink")).unwrap();
        mkfifo(&media.join("F/Keep/Deep/fifo"));
    });
    let progress = Recorder::default();

    move_entry(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    let (media, documents) = (fixture.path("media"), fixture.path("documents"));
    assert_eq!(names(&media.join("F")), ["Keep"]);
    assert_eq!(names(&media.join("F/Keep")), ["Deep", "dirlink"]);
    assert_eq!(names(&media.join("F/Keep/Deep")), ["fifo"]);
    assert_eq!(fs::read(media.join("Other/c.txt")).unwrap(), b"c");
    assert_eq!(fs::read(documents.join("F/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(documents.join("F/Gone/b.txt")).unwrap(), b"b");
    assert!(documents.join("F/Keep/Deep").is_dir());
    assert!(names(&documents.join("F/Keep/Deep")).is_empty());
    assert_eq!(progress.skipped.load(Ordering::Relaxed), 2);
}

#[test]
fn cross_volume_move_of_a_link_to_a_folder_removes_only_the_link() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("Real/a.txt"), b"a");
        write(&media.join("Real/Sub/b.txt"), b"b");
        symlink("Real", media.join("link")).unwrap();
    });
    let progress = Recorder::default();

    move_entry(
        fixture.media(),
        "link",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    )
    .unwrap();

    let (media, documents) = (fixture.path("media"), fixture.path("documents"));
    assert_eq!(names(&media), ["Real"]);
    assert_eq!(fs::read(media.join("Real/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(media.join("Real/Sub/b.txt")).unwrap(), b"b");
    assert_eq!(fs::read(documents.join("link/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(documents.join("link/Sub/b.txt")).unwrap(), b"b");
}

#[test]
fn move_into_descendant_is_refused() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("A/sub/x.txt"), b"x");
        symlink("A", media.join("link")).unwrap();
    });
    for dest_dir in ["A", "A/sub", "link/sub"] {
        let checked = storage::check_move(
            fixture.media(),
            &rel("A"),
            fixture.media(),
            &rel(dest_dir),
            ConflictPolicy::KeepBoth,
        );
        assert!(
            matches!(&checked, Err(StorageError::InvalidDestination(path)) if *path == rel("A")),
            "{dest_dir}: {checked:?}"
        );
    }

    let unchecked = storage::move_entry(
        fixture.media(),
        &rel("A"),
        fixture.media(),
        &rel("A/sub"),
        ConflictPolicy::KeepBoth,
        None,
        &Recorder::default(),
    );
    assert!(
        matches!(&unchecked, Err(StorageError::InvalidDestination(path)) if *path == rel("A")),
        "{unchecked:?}"
    );
    assert!(fixture.path("media").join("A/sub/x.txt").is_file());
}

#[test]
fn move_to_own_folder_is_refused() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("Sub/a.txt"), b"a");
        write(&media.join("top.txt"), b"t");
        symlink("Sub", media.join("link")).unwrap();
    });
    for (path, dest_dir) in [("Sub/a.txt", "Sub"), ("Sub/a.txt", "link"), ("top.txt", "")] {
        for conflict in [ConflictPolicy::Fail, ConflictPolicy::KeepBoth] {
            let checked = storage::check_move(
                fixture.media(),
                &rel(path),
                fixture.media(),
                &rel(dest_dir),
                conflict,
            );
            assert!(
                matches!(&checked, Err(StorageError::InvalidDestination(p)) if *p == rel(path)),
                "{path} → {dest_dir}: {checked:?}"
            );
        }
    }
    storage::check_move(
        fixture.media(),
        &rel("Sub/a.txt"),
        fixture.media(),
        &RelPath::root(),
        ConflictPolicy::Fail,
    )
    .unwrap();
}

#[test]
fn move_keep_both() {
    let fixture = Fixture::new(|media, documents| {
        write(&media.join("a.txt"), b"mine");
        write(&media.join("F/x.txt"), b"x");
        write(&media.join("Sub/a.txt"), b"theirs");
        write(&documents.join("F/y.txt"), b"y");
    });

    let same = move_entry(
        fixture.media(),
        "a.txt",
        fixture.media(),
        "Sub",
        ConflictPolicy::KeepBoth,
        &Recorder::default(),
    )
    .unwrap();
    let across = move_entry(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::KeepBoth,
        &Recorder::default(),
    )
    .unwrap();

    assert!(
        matches!(same, MoveOutcome::Renamed { final_path } if final_path == rel("Sub/a (1).txt"))
    );
    assert!(matches!(across, MoveOutcome::Copied { final_path } if final_path == rel("F (1)")));
    let (media, documents) = (fixture.path("media"), fixture.path("documents"));
    assert_eq!(fs::read(media.join("Sub/a.txt")).unwrap(), b"theirs");
    assert_eq!(fs::read(media.join("Sub/a (1).txt")).unwrap(), b"mine");
    assert_eq!(fs::read(documents.join("F (1)/x.txt")).unwrap(), b"x");
    assert!(documents.join("F/y.txt").is_file());
    assert!(!media.join("a.txt").exists() && !media.join("F").exists());
}

#[test]
fn move_replace_file() {
    let fixture = Fixture::new(|media, documents| {
        write(&media.join("a.txt"), b"new");
        write(&media.join("b.txt"), b"new b");
        write(&media.join("Sub/a.txt"), b"old");
        write(&documents.join("b.txt"), b"old b");
    });

    let same = move_entry(
        fixture.media(),
        "a.txt",
        fixture.media(),
        "Sub",
        ConflictPolicy::Replace,
        &Recorder::default(),
    )
    .unwrap();
    let across = move_entry(
        fixture.media(),
        "b.txt",
        fixture.documents(),
        "",
        ConflictPolicy::Replace,
        &Recorder::default(),
    )
    .unwrap();

    assert!(matches!(same, MoveOutcome::Renamed { final_path } if final_path == rel("Sub/a.txt")));
    assert!(matches!(across, MoveOutcome::Copied { final_path } if final_path == rel("b.txt")));
    let media = fixture.path("media");
    assert_eq!(fs::read(media.join("Sub/a.txt")).unwrap(), b"new");
    assert_eq!(
        fs::read(fixture.path("documents").join("b.txt")).unwrap(),
        b"new b"
    );
    assert_eq!(names(&media), ["Sub"]);
}

#[test]
fn move_target_appearing_after_check_is_kept() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("a.txt"), b"mine");
        write(&media.join("F/x.txt"), b"x");
        fs::create_dir(media.join("Sub")).unwrap();
    });
    for path in ["a.txt", "F"] {
        storage::check_move(
            fixture.media(),
            &rel(path),
            fixture.media(),
            &rel("Sub"),
            ConflictPolicy::Fail,
        )
        .unwrap();
    }
    let sub = fixture.path("media").join("Sub");
    write(&sub.join("a.txt"), b"theirs");
    write(&sub.join("F/theirs.txt"), b"theirs");

    for (path, expected) in [("a.txt", "Sub/a (1).txt"), ("F", "Sub/F (1)")] {
        let outcome = storage::move_entry(
            fixture.media(),
            &rel(path),
            fixture.media(),
            &rel("Sub"),
            ConflictPolicy::Fail,
            None,
            &Recorder::default(),
        )
        .unwrap();
        assert!(
            matches!(&outcome, MoveOutcome::Renamed { final_path } if *final_path == rel(expected)),
            "{outcome:?}"
        );
    }

    assert_eq!(fs::read(sub.join("a.txt")).unwrap(), b"theirs");
    assert_eq!(fs::read(sub.join("a (1).txt")).unwrap(), b"mine");
    assert_eq!(names(&sub.join("F")), ["theirs.txt"]);
    assert_eq!(fs::read(sub.join("F (1)/x.txt")).unwrap(), b"x");
}

#[test]
fn cancelled_cross_volume_move_keeps_the_source() {
    let fixture = Fixture::new(|media, _| {
        for name in ["a", "b", "c"] {
            write(&media.join(format!("F/{name}.txt")), name.as_bytes());
        }
    });
    let progress = Recorder {
        cancel_after_files: Some(2),
        ..Recorder::default()
    };

    let result = move_entry(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &progress,
    );

    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert_eq!(
        names(&fixture.path("media").join("F")),
        ["a.txt", "b.txt", "c.txt"]
    );
    assert!(!fixture.path("documents").join("F").exists());
    assert!(fixture.staged("documents").is_empty());
}

#[test]
fn cross_volume_move_failing_to_delete_keeps_both_copies() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/a.txt"), b"a");
        write(&media.join("F/b.txt"), b"b");
    });
    let source = fixture.path("media").join("F");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();

    let result = move_entry(
        fixture.media(),
        "F",
        fixture.documents(),
        "",
        ConflictPolicy::Fail,
        &Recorder::default(),
    );
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        matches!(&result, Err(StorageError::Source(inner))
            if matches!(**inner, StorageError::PermissionDenied(ref p) if p.as_str().starts_with("F/"))),
        "{result:?}"
    );
    assert_eq!(names(&source), ["a.txt", "b.txt"]);
    let documents = fixture.path("documents");
    assert_eq!(fs::read(documents.join("F/a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(documents.join("F/b.txt")).unwrap(), b"b");
    assert!(fixture.staged("documents").is_empty());
}

#[test]
fn planning_stops_once_cancelled() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/a.txt"), b"a");
        write(&media.join("F/Sub/b.txt"), b"b");
    });
    let progress = Recorder::default();
    progress.cancelled.store(true, Ordering::Relaxed);
    let planned = storage::plan_transfer(fixture.media(), &rel("F"), &progress);
    assert!(
        matches!(planned, Err(StorageError::Cancelled)),
        "{planned:?}"
    );
    let plan = storage::plan_transfer(fixture.media(), &rel("F"), &Recorder::default());
    assert_eq!(plan.unwrap().files, 2);
}
