//! Copies and moves whose source or destination changes while they run.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use explorer_server::storage::{self, RelPath, StorageError, TransferProgress, Volume, Volumes};
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
}

/// Runs `hook` with the count of files copied so far, after each file, and cancels from then on
/// if it answers `true`.
struct Hook<F: Fn(u64) -> bool> {
    files: AtomicU64,
    cancelled: AtomicBool,
    on_file: F,
}

impl<F: Fn(u64) -> bool> Hook<F> {
    fn new(hook: F) -> Self {
        Self {
            files: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            on_file: hook,
        }
    }
}

impl<F: Fn(u64) -> bool> TransferProgress for Hook<F> {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn add_bytes(&self, _n: u64) {}

    fn add_file(&self) {
        let files = self.files.fetch_add(1, Ordering::SeqCst) + 1;
        if (self.on_file)(files) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }

    fn skip(&self) {}
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn move_entry(
    fixture: &Fixture,
    path: &str,
    progress: &dyn TransferProgress,
) -> Result<storage::MoveOutcome, StorageError> {
    let (source, dest) = (fixture.media(), fixture.documents());
    storage::check_move(
        source,
        &rel(path),
        dest,
        &RelPath::root(),
        ConflictPolicy::Fail,
    )?;
    storage::move_entry(
        source,
        &rel(path),
        dest,
        &RelPath::root(),
        ConflictPolicy::Fail,
        None,
        progress,
    )
}

fn is_changed_during_move(result: &Result<storage::MoveOutcome, StorageError>, path: &str) -> bool {
    matches!(result, Err(StorageError::Source(inner))
        if matches!(&**inner, StorageError::ChangedDuringMove(changed) if changed.as_str() == path))
}

#[test]
fn cross_volume_move_keeps_a_source_file_replaced_after_its_copy() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("F/x.txt"), b"old");
        write(&media.join("F/y.txt"), b"y");
    });
    let source = fixture.path("media").join("F");
    // What an upload with `replace` does once the first file is copied: write, then rename.
    let progress = Hook::new(|files| {
        if files == 1 {
            write(&source.join(".up"), b"NEW VERSION");
            fs::rename(source.join(".up"), source.join("x.txt")).unwrap();
        }
        false
    });

    let result = move_entry(&fixture, "F", &progress);

    assert!(is_changed_during_move(&result, "F/x.txt"), "{result:?}");
    assert_eq!(read(&source.join("x.txt")), "NEW VERSION");
    assert!(!source.join("y.txt").exists());
    let documents = fixture.path("documents");
    assert_eq!(read(&documents.join("F/x.txt")), "old");
    assert_eq!(read(&documents.join("F/y.txt")), "y");
}

#[test]
fn cross_volume_move_keeps_a_source_file_written_after_its_copy() {
    let fixture = Fixture::new(|media, _| {
        write(&media.join("log.txt"), b"line1\n");
        write(&media.join("F/a.txt"), b"a");
        write(&media.join("F/b.txt"), b"b");
    });
    let media = fixture.path("media");
    let append = |path: &Path| {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(b"line2\n").unwrap();
    };

    let progress = Hook::new(|_| {
        append(&media.join("log.txt"));
        false
    });
    let result = move_entry(&fixture, "log.txt", &progress);
    assert!(is_changed_during_move(&result, "log.txt"), "{result:?}");
    assert_eq!(read(&media.join("log.txt")), "line1\nline2\n");
    assert_eq!(read(&fixture.path("documents").join("log.txt")), "line1\n");

    // A file of a folder: the others go, the changed one and its folder stay.
    let progress = Hook::new(|files| {
        if files == 2 {
            append(&media.join("F/a.txt"));
        }
        false
    });
    let result = move_entry(&fixture, "F", &progress);
    assert!(is_changed_during_move(&result, "F/a.txt"), "{result:?}");
    let left: Vec<_> = fs::read_dir(media.join("F"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["a.txt"]);
    assert_eq!(read(&media.join("F/a.txt")), "aline2\n");
}

#[test]
fn cancelled_copy_never_removes_a_folder_that_took_its_name() {
    let fixture = Fixture::new(|media, documents| {
        for name in ["a", "b", "c"] {
            write(&media.join(format!("src/{name}.txt")), name.as_bytes());
        }
        write(&documents.join("other/precious.txt"), b"precious");
    });
    let documents = fixture.path("documents");
    // The partly copied folder is renamed and another one takes its name, then the copy stops.
    let progress = Hook::new(|files| {
        if files == 1 {
            fs::rename(documents.join("src"), documents.join("src-partial")).unwrap();
            fs::rename(documents.join("other"), documents.join("src")).unwrap();
            return true;
        }
        false
    });

    let result = storage::copy(
        fixture.media(),
        &rel("src"),
        fixture.documents(),
        &RelPath::root(),
        ConflictPolicy::Fail,
        &storage::plan_transfer(fixture.media(), &rel("src")).unwrap(),
        &progress,
    );

    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert_eq!(read(&documents.join("src/precious.txt")), "precious");
}

#[test]
fn failed_copy_keeps_what_others_put_in_its_folder() {
    let fixture = Fixture::new(|media, _| {
        for name in ["a", "b", "c"] {
            write(&media.join(format!("F/sub/{name}.txt")), name.as_bytes());
        }
    });
    let documents = fixture.path("documents");
    let progress = Hook::new(|files| {
        if files == 1 {
            write(&documents.join("F/mine.txt"), b"mine");
            write(&documents.join("F/sub/mine.txt"), b"mine too");
            return true;
        }
        false
    });

    let result = storage::copy(
        fixture.media(),
        &rel("F"),
        fixture.documents(),
        &RelPath::root(),
        ConflictPolicy::Fail,
        &storage::plan_transfer(fixture.media(), &rel("F")).unwrap(),
        &progress,
    );

    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert_eq!(read(&documents.join("F/mine.txt")), "mine");
    assert_eq!(read(&documents.join("F/sub/mine.txt")), "mine too");
    let left: Vec<_> = fs::read_dir(documents.join("F/sub"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["mine.txt"]);
}
