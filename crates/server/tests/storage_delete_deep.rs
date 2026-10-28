//! Alone in its binary: it lowers the process's descriptor limit.

use std::fs;
use std::os::fd::OwnedFd;
use std::path::Path;

use explorer_server::storage::{RelPath, Volumes};
use rustix::fs::{Mode, OFlags};
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// Far more levels than descriptors allowed below, and more frames than a recursive delete fits
/// in the stack below.
const DEPTH: usize = 1000;

/// `top/d/d/…/d`, `DEPTH` levels, with a file every hundred levels; built one level at a time
/// since the path is longer than any path the kernel takes.
fn deep_chain(top: &Path) {
    fs::create_dir(top).unwrap();
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut fd: OwnedFd = fs::File::open(top).unwrap().into();
    for level in 0..DEPTH {
        rustix::fs::mkdirat(&fd, c"d", Mode::from_raw_mode(0o755)).unwrap();
        if level % 100 == 0 {
            let create = OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC;
            rustix::fs::openat(&fd, c"f", create, Mode::from_raw_mode(0o644)).unwrap();
        }
        fd = rustix::fs::openat(&fd, c"d", flags, Mode::empty()).unwrap();
    }
}

#[test]
fn deleting_a_very_deep_folder_needs_neither_a_deep_stack_nor_many_descriptors() {
    let data = tempfile::tempdir().unwrap();
    let media = data.path().join("media");
    fs::create_dir(&media).unwrap();
    fs::write(media.join("kept.txt"), "kept").unwrap();
    deep_chain(&media.join("deep"));
    let volumes = Volumes::discover(data.path()).unwrap();

    let limit = getrlimit(Resource::Nofile);
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(256),
            maximum: limit.maximum,
        },
    )
    .unwrap();
    let deleted = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn_scoped(scope, || {
                volumes
                    .get("media")
                    .unwrap()
                    .delete(&RelPath::parse("deep").unwrap())
            })
            .unwrap()
            .join()
            .unwrap()
    });
    setrlimit(Resource::Nofile, limit).unwrap();

    deleted.unwrap();
    assert!(fs::symlink_metadata(media.join("deep")).is_err());
    assert_eq!(fs::read_to_string(media.join("kept.txt")).unwrap(), "kept");
}
