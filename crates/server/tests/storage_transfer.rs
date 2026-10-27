use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use explorer_server::storage::{
    self, COPY_CHUNK, RelPath, STAGING_DIR, StorageError, TransferProgress, Volume, Volumes,
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
    let plan = storage::plan_transfer(source, &path)?;
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
    let plan = storage::plan_transfer(fixture.media(), &rel("Films/a.mkv")).unwrap();
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
    let plan = storage::plan_transfer(fixture.media(), &rel("F")).unwrap();
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
                Err(StorageError::InvalidDestination(_))
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
    let plan = storage::plan_transfer(fixture.media(), &rel("F")).unwrap();
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
        let plan = storage::plan_transfer(fixture.media(), &rel(path)).unwrap();
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
