use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use explorer_server::storage::{
    RelPath, STAGING_DIR, StorageError, TransferProgress, USAGE_TOP_FILES, UsageNodeView,
    UsageTree, Volumes, usage, usage_with_max_dirs,
};
use shared::api::usage::{UsageItem, UsageKind};
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

    fn media(&self) -> PathBuf {
        self.data.path().join("media")
    }

    fn scan(&self, path: &str) -> Result<UsageTree, StorageError> {
        usage(
            self.volumes.get("media").unwrap(),
            &rel(path),
            &Counter::default(),
        )
    }
}

#[derive(Default)]
struct Counter {
    cancelled: AtomicBool,
    files: AtomicU64,
    bytes: AtomicU64,
    skipped: AtomicU64,
}

impl TransferProgress for Counter {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn add_bytes(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }

    fn add_file(&self) {
        self.files.fetch_add(1, Ordering::Relaxed);
    }

    fn skip(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }
}

fn rel(raw: &str) -> RelPath {
    RelPath::parse(raw).unwrap()
}

fn write(path: &Path, len: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![7u8; len]).unwrap();
}

/// What the entry takes on disk, as `du` counts it.
fn allocated(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().blocks() * 512
}

fn allocated_all(root: &Path, names: &[&str]) -> u64 {
    names.iter().map(|name| allocated(&root.join(name))).sum()
}

fn view(tree: &UsageTree, path: &str) -> UsageNodeView {
    tree.view(&rel(path))
        .unwrap_or_else(|| panic!("{path} not scanned"))
}

fn item<'a>(view: &'a UsageNodeView, name: &str) -> &'a UsageItem {
    view.items
        .iter()
        .find(|item| item.name == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}

#[test]
fn usage_sums_allocated_sizes() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.bin"), 10_000);
        write(&media.join("Sub/b.bin"), 50_000);
        write(&media.join("Sub/Deep/c.bin"), 3_000);
    });
    let media = fixture.media();
    let sub = allocated_all(&media, &["Sub", "Sub/b.bin", "Sub/Deep", "Sub/Deep/c.bin"]);
    let total = allocated(&media) + allocated(&media.join("a.bin")) + sub;
    let file_bytes = allocated_all(&media, &["a.bin", "Sub/b.bin", "Sub/Deep/c.bin"]);

    let counter = Counter::default();
    let tree = usage(
        fixture.volumes.get("media").unwrap(),
        &RelPath::root(),
        &counter,
    )
    .unwrap();
    assert!(tree.root().is_root());
    assert_eq!(tree.totals(), (total, 3, 2));
    assert_eq!(counter.files.load(Ordering::Relaxed), 3);
    assert_eq!(counter.bytes.load(Ordering::Relaxed), file_bytes);
    assert_eq!(counter.skipped.load(Ordering::Relaxed), 0);

    let root = view(&tree, "");
    assert!(root.path.is_root());
    assert_eq!((root.bytes, root.files, root.dirs), (total, 3, 2));
    assert!(!root.incomplete && !root.collapsed);
    assert_eq!(root.items.len(), 2);
    let sub_item = item(&root, "Sub");
    assert_eq!(
        (sub_item.kind, sub_item.bytes, sub_item.files),
        (UsageKind::Dir, sub, 2)
    );
    let a = item(&root, "a.bin");
    assert_eq!(
        (a.kind, a.bytes, a.files),
        (UsageKind::File, allocated(&media.join("a.bin")), 1)
    );

    let deep = view(&tree, "Sub/Deep");
    assert_eq!(deep.path.as_str(), "Sub/Deep");
    assert_eq!((deep.files, deep.dirs), (1, 0));

    let from_sub = fixture.scan("Sub").unwrap();
    assert_eq!(from_sub.root().as_str(), "Sub");
    assert_eq!(from_sub.totals(), (sub, 2, 1));

    assert!(matches!(
        fixture.scan("a.bin"),
        Err(StorageError::NotADirectory(_))
    ));
    assert!(matches!(
        fixture.scan("nope"),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn usage_lists_biggest_first() {
    let fixture = Fixture::new(|media| {
        write(&media.join("small.bin"), 1_000);
        write(&media.join("big.bin"), 300_000);
        write(&media.join("Mid/m.bin"), 60_000);
    });
    let tree = fixture.scan("").unwrap();
    let root = view(&tree, "");
    let names: Vec<&str> = root.items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["big.bin", "Mid", "small.bin"]);
}

#[test]
fn usage_counts_hard_links_once() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.bin"), 100_000);
        fs::create_dir(media.join("Sub")).unwrap();
        fs::hard_link(media.join("a.bin"), media.join("Sub/b.bin")).unwrap();
        fs::hard_link(media.join("a.bin"), media.join("c.bin")).unwrap();
    });
    let media = fixture.media();
    let total = allocated_all(&media, &["", "a.bin", "Sub"]);
    let tree = fixture.scan("").unwrap();
    assert_eq!(tree.totals(), (total, 1, 1));
}

#[test]
fn usage_does_not_follow_symlinks() {
    let fixture = Fixture::new(|media| {
        write(&media.join("Big/huge.bin"), 300_000);
        fs::create_dir(media.join("Sub")).unwrap();
        symlink("Big", media.join("to-big")).unwrap();
        symlink(".", media.join("loop")).unwrap();
        symlink("..", media.join("Sub/back")).unwrap();
    });
    let media = fixture.media();
    let total = allocated_all(
        &media,
        &[
            "",
            "Big",
            "Big/huge.bin",
            "Sub",
            "to-big",
            "loop",
            "Sub/back",
        ],
    );
    let tree = fixture.scan("").unwrap();
    assert_eq!(tree.totals(), (total, 4, 2));
    let root = view(&tree, "");
    let link = item(&root, "to-big");
    assert_eq!(
        (link.kind, link.bytes),
        (UsageKind::File, allocated(&media.join("to-big")))
    );
    assert!(tree.view(&rel("to-big")).is_none());
    assert!(tree.view(&rel("loop/Big")).is_none());
}

#[test]
fn usage_marks_unreadable_folders() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    let fixture = Fixture::new(|media| {
        write(&media.join("Locked/x.bin"), 20_000);
        write(&media.join("Open/y.bin"), 20_000);
    });
    let media = fixture.media();
    let total = allocated_all(&media, &["", "Locked", "Open", "Open/y.bin"]);
    fs::set_permissions(media.join("Locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let counter = Counter::default();
    let result = usage(
        fixture.volumes.get("media").unwrap(),
        &RelPath::root(),
        &counter,
    );
    fs::set_permissions(media.join("Locked"), fs::Permissions::from_mode(0o755)).unwrap();

    let tree = result.unwrap();
    assert_eq!(tree.totals(), (total, 1, 2));
    assert_eq!(counter.skipped.load(Ordering::Relaxed), 1);
    let root = view(&tree, "");
    assert!(root.incomplete);
    assert!(item(&root, "Locked").incomplete);
    assert!(!item(&root, "Open").incomplete);
    assert!(view(&tree, "Locked").incomplete);
    assert!(!view(&tree, "Open").incomplete);
}

#[test]
fn usage_folds_small_files() {
    let count = USAGE_TOP_FILES + 5;
    let fixture = Fixture::new(|media| {
        for i in 0..count {
            write(&media.join(format!("f{i:02}.bin")), (i + 1) * 8192);
        }
    });
    let media = fixture.media();
    let mut sizes: Vec<u64> = (0..count)
        .map(|i| allocated(&media.join(format!("f{i:02}.bin"))))
        .collect();
    sizes.sort_unstable();

    let tree = fixture.scan("").unwrap();
    let root = view(&tree, "");
    assert_eq!(root.files, count as u64);
    assert_eq!(root.items.len(), USAGE_TOP_FILES + 1);
    let files = root
        .items
        .iter()
        .filter(|item| item.kind == UsageKind::File)
        .count();
    assert_eq!(files, USAGE_TOP_FILES);
    let other = root
        .items
        .iter()
        .find(|item| item.kind == UsageKind::OtherFiles)
        .unwrap();
    assert_eq!(other.files, 5);
    assert_eq!(other.bytes, sizes[..5].iter().sum::<u64>());
    assert!(root.items.windows(2).all(|w| w[0].bytes >= w[1].bytes));
}

#[test]
fn usage_collapses_beyond_the_folder_limit() {
    let fixture = Fixture::new(|media| {
        for name in ["A", "B", "C", "D"] {
            write(&media.join(format!("{name}/x.bin")), 10_000);
            write(&media.join(format!("{name}/Inner/y.bin")), 10_000);
        }
    });
    let media = fixture.media();
    let mut total = allocated(&media);
    for name in ["A", "B", "C", "D"] {
        total += allocated_all(&media.join(name), &["", "x.bin", "Inner", "Inner/y.bin"]);
    }
    let tree = usage_with_max_dirs(
        fixture.volumes.get("media").unwrap(),
        &RelPath::root(),
        &Counter::default(),
        3,
    )
    .unwrap();
    assert_eq!(tree.totals(), (total, 8, 8));

    let root = view(&tree, "");
    assert!(root.collapsed);
    let dirs: Vec<&UsageItem> = root
        .items
        .iter()
        .filter(|item| item.kind == UsageKind::Dir)
        .collect();
    assert_eq!(dirs.len(), 2);
    for dir in dirs {
        let inner = view(&tree, &dir.name);
        assert!(inner.collapsed, "{}", dir.name);
        assert_eq!(inner.dirs, 1);
        assert!(inner.items.iter().all(|item| item.kind != UsageKind::Dir));
    }
}

#[test]
fn usage_view_rejects_paths_outside() {
    let fixture = Fixture::new(|media| {
        write(&media.join("Sub/x.bin"), 1_000);
        write(&media.join("Sub/Deep/y.bin"), 1_000);
        write(&media.join("Other/z.bin"), 1_000);
    });
    let tree = fixture.scan("Sub").unwrap();
    for path in ["", "Other", "Sub/x.bin", "Sub/Missing", "Sub/Deep/y.bin"] {
        assert!(tree.view(&rel(path)).is_none(), "{path}");
    }
    assert_eq!(view(&tree, "Sub").path.as_str(), "Sub");
    assert_eq!(view(&tree, "Sub/Deep").files, 1);
}

#[test]
fn usage_stops_when_cancelled() {
    let fixture = Fixture::new(|media| write(&media.join("Sub/x.bin"), 1_000));
    let counter = Counter::default();
    counter.cancelled.store(true, Ordering::Relaxed);
    let result = usage(
        fixture.volumes.get("media").unwrap(),
        &RelPath::root(),
        &counter,
    );
    assert!(matches!(result, Err(StorageError::Cancelled)));
}

#[test]
fn usage_skips_staging() {
    let fixture = Fixture::new(|media| {
        write(&media.join("a.bin"), 1_000);
        write(&media.join(STAGING_DIR).join("upload"), 500_000);
    });
    let media = fixture.media();
    let total = allocated_all(&media, &["", "a.bin"]);
    let tree = fixture.scan("").unwrap();
    assert_eq!(tree.totals(), (total, 1, 0));
    assert!(
        view(&tree, "")
            .items
            .iter()
            .all(|item| item.name != STAGING_DIR)
    );
}

/// Panics on the first file it is told about.
struct Panicking;

impl TransferProgress for Panicking {
    fn is_cancelled(&self) -> bool {
        false
    }

    fn add_bytes(&self, _: u64) {}

    fn add_file(&self) {
        panic!("injected panic");
    }

    fn skip(&self) {}
}

#[test]
fn usage_fails_when_a_worker_panics() {
    // One folder: the other workers wait for the one that panics.
    let fixture = Fixture::new(|media| write(&media.join("x.bin"), 1_000));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let media = fixture.volumes.get("media").unwrap();
        let result = usage(media, &RelPath::root(), &Panicking);
        sender
            .send(matches!(result, Err(StorageError::Io { .. })))
            .unwrap();
    });
    let failed = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the scan hung after a worker panicked");
    assert!(failed);
}
