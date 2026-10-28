use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use ::zip::write::{FullFileOptions, StreamWriter};
use ::zip::{CompressionMethod, DateTime, ZipWriter};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::Response;
use axum_extra::extract::Query;
use serde::Deserialize;
use shared::api::downloads::ZipSelection;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::attachment_headers;
use super::channel::{CHUNK, ChannelWriter, Chunk, ZIP_SEND_TIMEOUT, send_timeout};
use crate::auth::Action;
use crate::auth::CurrentUser;
use crate::http::AppState;
use crate::http::access::{authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::{EntryKind, RelPath, SkipReason, TreeItem, TreeWalk, Volume};

const EXTENDED_TIMESTAMP: u16 = 0x5455;
const CHANNEL_CAPACITY: usize = 8;
/// Added at the top of an archive that left entries out, listing them.
pub const SKIPPED_LIST: &str = "explorer-skipped.txt";
/// Entries the list names; the rest are only counted.
const SKIPPED_LIST_LIMIT: usize = 10_000;

/// Query of `/files/zip`: with `name`s, only those entries of `path` are archived.
#[derive(Deserialize)]
pub struct ZipQuery {
    volume: String,
    path: String,
    #[serde(default)]
    name: Vec<String>,
}

/// Query of `/files/zip` for a stored selection.
#[derive(Deserialize)]
struct SelectionQuery {
    selection: String,
}

/// Answers `HEAD` without walking the tree, which `get` would otherwise do implicitly.
pub async fn head() -> Response {
    let mut res = Response::new(Body::empty());
    *res.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
    res.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET"));
    res
}

pub async fn zip(
    State(state): State<AppState>,
    user: CurrentUser,
    uri: Uri,
) -> Result<Response, ApiError> {
    let selection = if let Ok(Query(query)) = Query::<SelectionQuery>::try_from_uri(&uri) {
        state
            .downloads
            .selection(&user.sub, &query.selection)
            .ok_or_else(ApiError::selection_expired)?
    } else {
        let Ok(Query(query)) = Query::<ZipQuery>::try_from_uri(&uri) else {
            return Err(invalid_query());
        };
        ZipSelection {
            volume: query.volume,
            path: query.path,
            names: query.name,
        }
    };
    stream_zip(state, user, selection).await
}

/// Checks the request, then streams the archive while it walks the tree. Each zip holds one
/// of [`super::ZIP_CONCURRENCY`] slots until it ends, and stops when the client goes away.
async fn stream_zip(
    state: AppState,
    user: CurrentUser,
    ZipSelection {
        volume,
        path,
        mut names,
    }: ZipSelection,
) -> Result<Response, ApiError> {
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.clone()));
    let path = authorize(&state, &user, &volume, &path, Action::Read)?;
    let children = names
        .iter()
        .map(|name| path.join(name))
        .collect::<Result<Vec<_>, _>>()?;
    let permit = state
        .downloads
        .try_zip()
        .ok_or_else(ApiError::too_many_zips)?;

    let walk = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| {
            if children.is_empty() {
                return vol.tree_walk(&path, None);
            }
            let mut names = Vec::new();
            for child in &children {
                if vol.stat(child)?.kind == EntryKind::Other {
                    return Err(crate::storage::StorageError::NotFound(child.clone()));
                }
                names.extend(child.file_name().map(str::to_owned));
            }
            vol.tree_walk(&path, Some(&names))
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;

    let archive_name = format!("{}.zip", path.file_name().unwrap_or(&volume));
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let volumes = Arc::clone(&state.volumes);
    let span = tracing::Span::current();
    let names = names.len();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _entered = span.enter();
        let Some(vol) = volumes.get(&volume) else {
            let _ = tx.blocking_send(Err(io::Error::other("volume not found")));
            return;
        };
        let gone = tx.clone();
        let mut tally = Tally::default();
        let completed = stream_archive(&volume, &path, tx, |zip| {
            write_entries(zip, vol, walk, &|| gone.is_closed(), &mut tally)
        });
        tracing::info!(
            user = user.sub,
            action = "zip",
            volume,
            path = path.as_str(),
            files = tally.files,
            bytes = tally.bytes,
            skipped = tally.skipped_total,
            names,
            completed,
            "zip"
        );
    });

    let mut res = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    attachment_headers(res.headers_mut(), &archive_name);
    Ok(res)
}

type Archive = ZipWriter<StreamWriter<ChannelWriter>>;

struct Failure {
    path: String,
    error: Box<dyn std::error::Error + Send + Sync>,
}

impl Failure {
    fn at(path: &str, error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self {
            path: path.to_owned(),
            error: error.into(),
        }
    }
}

/// What an archive holds and what it left out.
#[derive(Default)]
struct Tally {
    files: u64,
    bytes: u64,
    skipped: Vec<(String, SkipReason)>,
    skipped_total: u64,
}

/// Ends the body with an error on any failure, unless the client is already gone. True when
/// the archive was sent whole.
fn stream_archive(
    volume: &str,
    root: &RelPath,
    tx: mpsc::Sender<Chunk>,
    entries: impl FnOnce(&mut Archive) -> Result<(), Failure>,
) -> bool {
    let errors = tx.clone();
    let Err(failure) = write_archive(ChannelWriter::new(tx), root, entries) else {
        return true;
    };
    if errors.is_closed() {
        tracing::debug!(volume, path = root.as_str(), "zip client went away");
        return false;
    }
    tracing::warn!(
        volume,
        path = root.as_str(),
        entry = failure.path,
        error = %failure.error,
        "zip aborted"
    );
    let _ = send_timeout(
        &errors,
        Err(io::Error::other("zip archive aborted")),
        ZIP_SEND_TIMEOUT,
    );
    false
}

fn write_archive(
    sink: ChannelWriter,
    root: &RelPath,
    entries: impl FnOnce(&mut Archive) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let aborted = Arc::clone(&sink.aborted);
    let mut zip = ZipWriter::new_stream(sink);
    if let Err(failure) = entries(&mut zip) {
        // Dropping the writer finalizes the archive; none of that footer may reach the client.
        aborted.store(true, Ordering::Relaxed);
        return Err(failure);
    }
    zip.finish()
        .and_then(|sink| Ok(sink.into_inner().flush()?))
        .map_err(|err| {
            aborted.store(true, Ordering::Relaxed);
            Failure::at(root.as_str(), err)
        })
}

/// Writes what the walk yields until it ends or `gone` says the client left, then the list of
/// skipped entries if there are any.
fn write_entries(
    zip: &mut Archive,
    volume: &Volume,
    mut walk: TreeWalk,
    gone: &dyn Fn() -> bool,
    tally: &mut Tally,
) -> Result<(), Failure> {
    while let Some(item) = walk.next(volume) {
        if gone() {
            return Err(Failure::at("", io::Error::from(io::ErrorKind::BrokenPipe)));
        }
        match item {
            TreeItem::Dir { name, modified } => {
                zip.add_directory(name.as_str(), entry_options(modified))
                    .map_err(|err| Failure::at(&name, err))?;
            }
            TreeItem::File {
                name,
                file,
                size,
                modified,
            } => {
                zip.start_file(name.as_str(), file_options(modified))
                    .map_err(|err| Failure::at(&name, err))?;
                copy_data(zip, &name, file)?;
                tally.files += 1;
                tally.bytes += size;
            }
            TreeItem::Skipped { name, reason } => {
                tracing::debug!(
                    entry = name,
                    reason = reason.describe(),
                    "zip skips an entry"
                );
                tally.skipped_total += 1;
                if tally.skipped.len() < SKIPPED_LIST_LIMIT {
                    tally.skipped.push((name, reason));
                }
            }
        }
    }
    if tally.skipped_total == 0 {
        return Ok(());
    }
    tracing::warn!(skipped = tally.skipped_total, "zip left entries out");
    let name = walk.unused_top_name(SKIPPED_LIST);
    zip.start_file(name.as_str(), file_options(Some(SystemTime::now())))
        .map_err(|err| Failure::at(&name, err))?;
    zip.write_all(skipped_list(tally).as_bytes())
        .map_err(|err| Failure::at(&name, err))
}

fn skipped_list(tally: &Tally) -> String {
    let mut list = String::from("These entries could not be put in the archive:\n\n");
    for (name, reason) in &tally.skipped {
        let _ = writeln!(list, "{name}: {}", reason.describe());
    }
    let more = tally.skipped_total - tally.skipped.len() as u64;
    if more > 0 {
        let _ = writeln!(list, "... and {more} more");
    }
    list
}

fn copy_data(zip: &mut Archive, name: &str, mut source: impl Read) -> Result<(), Failure> {
    let mut buf = vec![0; CHUNK];
    loop {
        let n = match source.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(Failure::at(name, err)),
        };
        zip.write_all(&buf[..n])
            .map_err(|err| Failure::at(name, err))?;
    }
}

fn file_options(modified: Option<SystemTime>) -> FullFileOptions<'static, 'static> {
    entry_options(modified)
        .compression_method(CompressionMethod::Stored)
        .large_file(true)
}

/// The DOS time of an entry is naive and in UTC; an extended timestamp gives extractors that
/// read it the exact time, which they show in their own time zone.
fn entry_options(modified: Option<SystemTime>) -> FullFileOptions<'static, 'static> {
    let mut options = FullFileOptions::default();
    let Some(modified) = modified else {
        return options;
    };
    if let Some(time) = zip_time(modified) {
        options = options.last_modified_time(time);
    }
    if let Some(field) = extended_timestamp(modified) {
        // The field is 5 bytes, far below the limit on extra data.
        let _ = options.add_extra_field(EXTENDED_TIMESTAMP, field, false);
    }
    options
}

/// The modification time as an "UT" extra field: a flag byte, then signed Unix seconds.
fn extended_timestamp(time: SystemTime) -> Option<[u8; 5]> {
    let secs = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i32::try_from(after.as_secs()).ok()?,
        Err(before) => i32::try_from(before.duration().as_secs())
            .ok()?
            .checked_neg()?,
    };
    let mut field = [1, 0, 0, 0, 0];
    field[1..].copy_from_slice(&secs.to_le_bytes());
    Some(field)
}

/// Zip timestamps are naive; UTC is used. Times outside 1980..=2107 are dropped.
fn zip_time(time: SystemTime) -> Option<DateTime> {
    let secs = i64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let utc = time::OffsetDateTime::from_unix_timestamp(secs).ok()?;
    DateTime::from_date_and_time(
        u16::try_from(utc.year()).ok()?,
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second(),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// The writer blocks on the runtime it runs under, as on a blocking thread of the server.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    /// Yields `0` bytes of ones, then fails.
    struct FailsAfter(usize);

    impl Read for FailsAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                return Err(io::Error::other("disk error"));
            }
            let n = buf.len().min(self.0);
            buf[..n].fill(1);
            self.0 -= n;
            Ok(n)
        }
    }

    /// The failure points sweep a chunk boundary, so the footer the dropped writer emits would
    /// fill a chunk and be sent unless the abort comes first.
    #[test]
    fn failure_mid_file_sends_no_footer_then_an_error() {
        let rt = runtime();
        let _entered = rt.enter();
        for short in 0..512 {
            let (tx, mut rx) = mpsc::channel(64);
            let path = "big.bin";
            stream_archive("media", &RelPath::root(), tx, |zip| {
                zip.start_file(path, file_options(None))
                    .map_err(|err| Failure::at(path, err))?;
                copy_data(zip, path, FailsAfter(3 * CHUNK - short))
            });
            let items: Vec<Chunk> = std::iter::from_fn(|| rx.blocking_recv()).collect();
            let (last, data) = items.split_last().unwrap();
            assert!(last.is_err(), "no error at {short}");
            let bytes: Vec<u8> = data
                .iter()
                .flat_map(|chunk| chunk.as_ref().unwrap().to_vec())
                .collect();
            assert!(bytes.len() > CHUNK);
            assert!(
                !bytes.windows(4).any(|w| w == b"PK\x05\x06"),
                "footer sent at {short}"
            );
        }
    }

    #[test]
    fn client_gone_mid_file_aborts_the_writer() {
        let rt = runtime();
        let _entered = rt.enter();
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let path = "big.bin";
        let sink = ChannelWriter::new(tx);
        let aborted = Arc::clone(&sink.aborted);
        let result = write_archive(sink, &RelPath::root(), |zip| {
            zip.start_file(path, file_options(None))
                .map_err(|err| Failure::at(path, err))?;
            copy_data(zip, path, FailsAfter(3 * CHUNK))
        });
        assert!(result.is_err());
        assert!(aborted.load(Ordering::Relaxed));
    }

    #[test]
    fn mtime_outside_zip_range_is_dropped() {
        assert!(zip_time(UNIX_EPOCH).is_none());
        let time = zip_time(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)).unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (2023, 11, 14));
    }

    #[test]
    fn extended_timestamp_holds_signed_unix_seconds() {
        let after = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(extended_timestamp(after), Some([1, 0x00, 0xf1, 0x53, 0x65]));
        let before = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(
            extended_timestamp(before),
            Some([1, 0xff, 0xff, 0xff, 0xff])
        );
        assert_eq!(
            extended_timestamp(UNIX_EPOCH + Duration::from_secs(1 << 31)),
            None
        );
    }
}
