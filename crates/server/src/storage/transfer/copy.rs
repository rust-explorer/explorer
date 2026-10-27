use std::io::{self, Read, Write};
use std::time::SystemTime;

use shared::api::conflict::ConflictPolicy;

use super::{COPY_CHUNK, TransferPlan, TransferProgress};
use crate::storage::write::KEEP_BOTH_MAX;
use crate::storage::{
    InvalidPath, RelPath, StagedId, Staging, StorageError, Volume, WalkItem, keep_both_name,
};

/// Copies `path` of `source` into the folder `dest_dir` of `dest`, which may be the same volume,
/// following `plan`. Returns the path the copy ended up at. On failure or cancellation nothing
/// is left behind: no staged file, no partial folder.
pub fn copy(
    source: &Volume,
    path: &RelPath,
    dest: &Volume,
    dest_dir: &RelPath,
    conflict: ConflictPolicy,
    plan: &TransferPlan,
    progress: &dyn TransferProgress,
) -> Result<RelPath, StorageError> {
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
        return files.copy(file, &dest_dir.join(name)?, conflict, *modified);
    }

    let top = create_top(dest, dest_dir, name, conflict)?;
    match files.copy_tree(path, &top, plan) {
        Ok(()) => Ok(top),
        Err(err) => {
            if let Err(cleanup) = dest.delete(&top) {
                tracing::warn!(
                    volume = dest.name(),
                    path = top.as_str(),
                    error = %cleanup,
                    "cannot remove a partly copied folder"
                );
            }
            Err(err)
        }
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

impl Files<'_> {
    /// Replays the walk of `path` under the freshly created `top`, where nothing can conflict.
    fn copy_tree(
        &self,
        path: &RelPath,
        top: &RelPath,
        plan: &TransferPlan,
    ) -> Result<(), StorageError> {
        for item in &plan.items {
            if self.progress.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            match item {
                WalkItem::Dir(dir) => self.dest.create_dir(&rebase(dir, path, top)?)?,
                WalkItem::File {
                    path: file,
                    modified,
                    ..
                } => {
                    self.copy(
                        file,
                        &rebase(file, path, top)?,
                        ConflictPolicy::Fail,
                        *modified,
                    )?;
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
    ) -> Result<RelPath, StorageError> {
        let id = StagedId::random();
        let placed = self.stream(from, &id).and_then(|()| {
            if self.progress.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            self.dest
                .commit_staged(self.staging, &id, to, conflict, modified)
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

    fn stream(&self, from: &RelPath, id: &StagedId) -> Result<(), StorageError> {
        let read_err = |err| StorageError::from_io(from, err).on_source();
        let (mut input, _) = self
            .source
            .open_file(from)
            .map_err(StorageError::on_source)?;
        let mut output = self.staging.create(id)?;
        let mut chunk = vec![0; COPY_CHUNK];
        loop {
            if self.progress.is_cancelled() {
                return Err(StorageError::Cancelled);
            }
            let n = fill(&mut input, &mut chunk).map_err(read_err)?;
            if n == 0 {
                return Ok(());
            }
            output
                .write_all(&chunk[..n])
                .map_err(|err| StorageError::from_io(&RelPath::root(), err))?;
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
