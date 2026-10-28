use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::time::SystemTime;

use cap_std::fs::MetadataExt as _;
use shared::api::conflict::ConflictPolicy;

use super::mode::{kept_mode, set_dir_mode, set_file_mode};
use super::remove::{Expect, Removed, Stamp, remove_if};
use super::{COPY_CHUNK, TransferPlan, TransferProgress};
use crate::storage::read::Identity;
use crate::storage::write::KEEP_BOTH_MAX;
use crate::storage::{
    InvalidPath, RelPath, StagedId, Staging, StorageError, Volume, WalkItem, keep_both_name,
};

/// Copies `path` of `source` into the folder `dest_dir` of `dest`, which may be the same volume,
/// following `plan`. Returns the path the copy ended up at. On failure or cancellation nothing
/// it made is left behind: no staged file, no partial folder, unless something else was put
/// there meanwhile.
pub fn copy(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
    plan: &TransferPlan,
    progress: &dyn TransferProgress,
) -> Result<RelPath, StorageError> {
    copy_taking(source, path, dest, dest_dir, conflict, plan, progress).map(|(placed, _)| placed)
}

/// What a copy read from its source, for a move to remove: the copied entry, and the files and
/// folders of a copied folder by their index in the plan, each as it was when read.
pub(super) struct Taken {
    pub top: Expect,
    pub files: Vec<(usize, Stamp)>,
    pub dirs: Vec<(usize, Identity)>,
}

/// What a folder copy made in the destination, by index in the plan, with the path a file
/// ended up at when it is not the planned one.
/// Each copied folder also comes with its source's permissions, applied once the copy is done.
#[derive(Default)]
struct Made {
    entries: Vec<(usize, Option<RelPath>, Expect)>,
    dir_modes: Vec<(RelPath, Identity, u32)>,
}

/// Like [`copy`], also returning what it took from the source.
pub(super) fn copy_taking(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
    plan: &TransferPlan,
    progress: &dyn TransferProgress,
) -> Result<(RelPath, Taken), StorageError> {
    let Some(name) = path.file_name() else {
        return Err(InvalidPath {
            reason: "the volume root can't be copied",
        }
        .into());
    };
    for _ in 0..plan.skipped {
        progress.skip();
    }
    let staging = dest.staging()?;
    let files = Files {
        source,
        dest,
        staging: &staging,
        progress,
    };
    if let [
        WalkItem::File {
            path: file,
            modified,
            ..
        },
    ] = plan.items.as_slice()
        && file == path
    {
        let copied = files.copy(file, &dest_dir.join(name)?, conflict, *modified)?;
        let taken = Taken {
            top: Expect::File(copied.source),
            files: Vec::new(),
            dirs: Vec::new(),
        };
        return Ok((copied.placed, taken));
    }

    let source_err = |err| StorageError::from_io(path, err).on_source();
    let top_meta = source.root.metadata(path.as_str()).map_err(source_err)?;
    let top_id = Identity::of(&top_meta);
    let top = create_top(dest, dest_dir, name, conflict)?;
    let made_top = dest
        .root
        .symlink_metadata(top.as_str())
        .map(|meta| Identity::of(&meta))
        .map_err(|err| StorageError::from_io(&top, err))?;
    let mut taken = Taken {
        top: Expect::Dir(top_id),
        files: Vec::new(),
        dirs: Vec::new(),
    };
    let mut made = Made::default();
    match files.copy_tree(path, &top, plan, &mut made, &mut taken) {
        Ok(()) => {
            for (dir, id, mode) in made.dir_modes.iter().rev() {
                set_dir_mode(dest, dir, *id, *mode);
            }
            set_dir_mode(dest, &top, made_top, kept_mode(top_meta.mode()));
            Ok((top, taken))
        }
        Err(err) => {
            undo(dest, path, &top, made_top, plan, &made);
            Err(err)
        }
    }
}

/// Removes what a failed folder copy made, deepest first, each only if it is still what the
/// copy made: whatever else was put there meanwhile stays, with the folders holding it.
fn undo(
    dest: &Volume,
    path: &RelPath,
    top: &RelPath,
    made_top: Identity,
    plan: &TransferPlan,
    made: &Made,
) {
    let remove = |at: &RelPath, expect| match remove_if(dest, at, expect, false) {
        Ok(Removed::Yes | Removed::Gone) => 0,
        Ok(Removed::Changed | Removed::NotEmpty) => 1,
        Err(err) => {
            tracing::warn!(volume = dest.name(), path = at.as_str(), error = %err, "cannot remove part of a failed copy");
            1
        }
    };
    let mut kept = 0;
    for (index, placed, expect) in made.entries.iter().rev() {
        let planned = match &plan.items[*index] {
            WalkItem::Dir(item) | WalkItem::File { path: item, .. } => rebase(item, path, top),
        };
        kept += match placed.clone().map_or(planned, Ok) {
            Ok(at) => remove(&at, *expect),
            Err(_) => 1,
        };
    }
    kept += remove(top, Expect::Dir(made_top));
    if kept > 0 {
        tracing::warn!(
            volume = dest.name(),
            path = top.as_str(),
            kept,
            "a failed copy left entries that changed or were added meanwhile"
        );
    }
}

/// Creates the copied folder under the first free name: no folder of the destination ever gets
/// merged into, whatever appeared since the check.
fn create_top(
    dest: &Volume,
    dest_dir: &RelPath,
    name: &str,
    conflict: ConflictPolicy,
) -> Result<RelPath, StorageError> {
    let wanted = dest_dir.join(name)?;
    for n in 0..=KEEP_BOTH_MAX {
        let candidate = if n == 0 {
            wanted.clone()
        } else {
            dest_dir.join(&keep_both_name(name, n))?
        };
        match dest.create_dir(&candidate) {
            Ok(()) => {
                if n > 0 && conflict == ConflictPolicy::Fail {
                    tracing::info!(
                        volume = dest.name(),
                        path = wanted.as_str(),
                        final_path = candidate.as_str(),
                        "destination appeared meanwhile; keeping both"
                    );
                }
                return Ok(candidate);
            }
            Err(StorageError::AlreadyExists(_)) if conflict != ConflictPolicy::Replace => {}
            Err(StorageError::AlreadyExists(_)) => {
                return Err(StorageError::IsADirectory(candidate));
            }
            Err(err) => return Err(err),
        }
    }
    Err(StorageError::AlreadyExists(wanted))
}

/// Streams files from `source` to staged files of `dest`, then into place.
struct Files<'a> {
    source: &'a Volume,
    dest: &'a Volume,
    staging: &'a Staging,
    progress: &'a dyn TransferProgress,
}

/// A file copied: where it was placed, the source as it was read and the copy as placed.
struct CopiedFile {
    placed: RelPath,
    source: Stamp,
    made: Stamp,
}

impl Files<'_> {
    /// Replays the walk of `path` under the freshly created `top`, where nothing can conflict,
    /// noting what it makes and what it takes.
    fn copy_tree(
        &self,
        path: &RelPath,
        top: &RelPath,
        plan: &TransferPlan,
        made: &mut Made,
        taken: &mut Taken,
    ) -> Result<(), StorageError> {
        for (index, item) in plan.items.iter().enumerate() {
            if self.progress.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            match item {
                WalkItem::Dir(dir) => {
                    let to = rebase(dir, path, top)?;
                    let source = self
                        .source
                        .root
                        .symlink_metadata(dir.as_str())
                        .ok()
                        .filter(cap_std::fs::Metadata::is_dir);
                    self.dest.create_dir(&to)?;
                    let created = self
                        .dest
                        .root
                        .symlink_metadata(to.as_str())
                        .map_err(|err| StorageError::from_io(&to, err))?;
                    let created = Identity::of(&created);
                    made.entries.push((index, None, Expect::Dir(created)));
                    if let Some(source) = source {
                        taken.dirs.push((index, Identity::of(&source)));
                        made.dir_modes.push((to, created, kept_mode(source.mode())));
                    }
                }
                WalkItem::File {
                    path: file,
                    modified,
                    ..
                } => {
                    let to = rebase(file, path, top)?;
                    let copied = self.copy(file, &to, ConflictPolicy::Fail, *modified)?;
                    let renamed = (copied.placed != to).then_some(copied.placed);
                    made.entries
                        .push((index, renamed, Expect::File(copied.made)));
                    taken.files.push((index, copied.source));
                }
            }
        }
        Ok(())
    }

    /// The file never reaches `to` unless it was copied whole.
    fn copy(
        &self,
        from: &RelPath,
        to: &RelPath,
        conflict: ConflictPolicy,
        modified: Option<SystemTime>,
    ) -> Result<CopiedFile, StorageError> {
        let id = StagedId::random();
        let placed = self
            .stream(from, to, &id)
            .and_then(|(output, source, mode)| {
                if self.progress.is_cancelled() {
                    return Err(StorageError::Cancelled);
                }
                set_file_mode(&output, mode, false, self.dest);
                let placed = self
                    .dest
                    .commit_staged(self.staging, &id, to, conflict, modified)?;
                set_file_mode(&output, mode, true, self.dest);
                let made = output
                    .metadata()
                    .map_err(|err| StorageError::from_io(&placed, err))?;
                Ok(CopiedFile {
                    placed,
                    source,
                    made: Stamp::placed(&made),
                })
            });
        match placed {
            Ok(placed) => {
                self.progress.add_file();
                Ok(placed)
            }
            Err(err) => {
                if let Err(cleanup) = self.staging.remove(&id) {
                    tracing::warn!(
                        volume = self.dest.name(),
                        error = %cleanup,
                        "cannot remove a staged copy"
                    );
                }
                Err(err)
            }
        }
    }

    /// Returns the staged file, still open, the source as it was when opened and the
    /// permissions the copy keeps.
    fn stream(
        &self,
        from: &RelPath,
        to: &RelPath,
        id: &StagedId,
    ) -> Result<(std::fs::File, Stamp, u32), StorageError> {
        let read_err = |err| StorageError::from_io(from, err).on_source();
        // Before the open: a symlink replaced after it is another inode.
        let entry = self.source.root.symlink_metadata(from.as_str()).ok();
        let (mut input, _) = self
            .source
            .open_file(from)
            .map_err(StorageError::on_source)?;
        let meta = input.metadata().map_err(read_err)?;
        let (source, mode) = (
            Stamp::source(&meta, entry.as_ref()),
            kept_mode(meta.permissions().mode()),
        );
        let mut output = self.staging.create(id)?;
        let mut chunk = vec![0; COPY_CHUNK];
        loop {
            if self.progress.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            let n = fill(&mut input, &mut chunk).map_err(read_err)?;
            if n == 0 {
                return Ok((output, source, mode));
            }
            output
                .write_all(&chunk[..n])
                .map_err(|err| StorageError::from_io(to, err))?;
            self.progress.add_bytes(n as u64);
        }
    }
}

/// Reads until `buf` is full or the file ends.
fn fill(input: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match input.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

/// `F/a/b` of the copied `F` → `<top>/a/b`.
fn rebase(item: &RelPath, path: &RelPath, top: &RelPath) -> Result<RelPath, StorageError> {
    let rest = item
        .as_str()
        .strip_prefix(path.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or(InvalidPath {
            reason: "a walked entry outside its folder",
        })?;
    Ok(RelPath::parse(&format!("{}/{rest}", top.as_str()))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_under_the_copy() {
        let rel = |raw| RelPath::parse(raw).unwrap();
        assert_eq!(
            rebase(&rel("a/F/x/y.txt"), &rel("a/F"), &rel("b/F (1)")).unwrap(),
            rel("b/F (1)/x/y.txt")
        );
        assert!(rebase(&rel("a/G/x"), &rel("a/F"), &rel("b/F")).is_err());
        assert!(rebase(&rel("a/Fx/y"), &rel("a/F"), &rel("b/F")).is_err());
    }

    #[test]
    fn fill_reads_a_whole_chunk() {
        let mut input = io::Cursor::new(vec![1u8; 10]).chain(io::Cursor::new(vec![2u8; 10]));
        let mut buf = [0u8; 15];
        assert_eq!(fill(&mut input, &mut buf).unwrap(), 15);
        assert_eq!(fill(&mut input, &mut buf).unwrap(), 5);
        assert_eq!(fill(&mut input, &mut buf).unwrap(), 0);
    }
}
