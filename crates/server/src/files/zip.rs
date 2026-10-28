use std::io::{self, Read, Write};
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ::zip::write::{SimpleFileOptions, StreamWriter};
use ::zip::{CompressionMethod, DateTime, ZipWriter};
use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::attachment_headers;
use crate::api::blocking_volume;
use crate::app::AppState;
use crate::auth::CurrentUser;
use crate::authz::{Action, Policy};
use crate::error::ApiError;
use crate::storage::{RelPath, Volume, WalkItem};

const CHUNK: usize = 64 * 1024;
const CHANNEL_CAPACITY: usize = 8;

type Chunk = io::Result<Bytes>;

#[derive(Deserialize)]
pub struct ZipQuery {
    volume: String,
    #[serde(default)]
    path: String,
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
    query: Result<Query<ZipQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Ok(Query(ZipQuery { volume, path })) = query else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_path",
            "The volume parameter is required",
        ));
    };
    if state.volumes.get(&volume).is_none() {
        return Err(ApiError::volume_not_found(&volume));
    }
    Policy.check(&user, &volume, Action::Read)?;
    let path = RelPath::parse(&path)?;

    let items = blocking_volume(&state.volumes, &volume, {
        let path = path.clone();
        move |vol| vol.walk(&path)
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;

    let (files, bytes) = items.iter().fold((0u64, 0u64), |(files, bytes), item| {
        if let WalkItem::File { size, .. } = item {
            (files + 1, bytes + size)
        } else {
            (files, bytes)
        }
    });
    tracing::info!(
        user = user.sub,
        action = "zip",
        volume,
        path = path.as_str(),
        files,
        bytes,
        "zip"
    );

    let archive_name = format!("{}.zip", path.file_name().unwrap_or(&volume));
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let volumes = Arc::clone(&state.volumes);
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        match volumes.get(&volume) {
            Some(vol) => stream_archive(&volume, &path, tx, |zip| {
                write_entries(zip, vol, &path, &items)
            }),
            None => {
                let _ = tx.blocking_send(Err(io::Error::other("volume not found")));
            }
        }
    });

    let mut res = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    attachment_headers(res.headers_mut(), &archive_name);
    Ok(res)
}

type Archive = ZipWriter<StreamWriter<ChannelWriter>>;

struct Failure {
    path: RelPath,
    error: Box<dyn std::error::Error + Send + Sync>,
}

impl Failure {
    fn at(path: &RelPath, error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self {
            path: path.clone(),
            error: error.into(),
        }
    }
}

/// Ends the body with an error on any failure, unless the client is already gone.
fn stream_archive(
    volume: &str,
    root: &RelPath,
    tx: mpsc::Sender<Chunk>,
    entries: impl FnOnce(&mut Archive) -> Result<(), Failure>,
) {
    let errors = tx.clone();
    let Err(failure) = write_archive(ChannelWriter::new(tx), root, entries) else {
        return;
    };
    if errors.is_closed() {
        tracing::debug!(volume, path = root.as_str(), "zip client went away");
        return;
    }
    tracing::warn!(
        volume,
        path = failure.path.as_str(),
        error = %failure.error,
        "zip aborted"
    );
    let _ = errors.blocking_send(Err(io::Error::other("zip archive aborted")));
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
            Failure::at(root, err)
        })
}

fn write_entries(
    zip: &mut Archive,
    volume: &Volume,
    root: &RelPath,
    items: &[WalkItem],
) -> Result<(), Failure> {
    for item in items {
        match item {
            WalkItem::Dir(path) => {
                zip.add_directory(entry_name(root, path), SimpleFileOptions::default())
                    .map_err(|err| Failure::at(path, err))?;
            }
            WalkItem::File { path, modified, .. } => {
                let (file, _) = volume
                    .open_file(path)
                    .map_err(|err| Failure::at(path, err))?;
                zip.start_file(entry_name(root, path), file_options(*modified))
                    .map_err(|err| Failure::at(path, err))?;
                copy_data(zip, path, file)?;
            }
        }
    }
    Ok(())
}

fn copy_data(zip: &mut Archive, path: &RelPath, mut source: impl Read) -> Result<(), Failure> {
    let mut buf = vec![0; CHUNK];
    loop {
        let n = match source.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(Failure::at(path, err)),
        };
        zip.write_all(&buf[..n])
            .map_err(|err| Failure::at(path, err))?;
    }
}

/// Backslashes become `_` so that Windows extractors cannot read them as separators.
fn entry_name(root: &RelPath, path: &RelPath) -> String {
    let relative = if root.is_root() {
        path.as_str()
    } else {
        path.as_str()
            .strip_prefix(root.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path.as_str())
    };
    relative.replace('\\', "_")
}

fn file_options(modified: Option<SystemTime>) -> SimpleFileOptions {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true);
    match modified.and_then(zip_time) {
        Some(time) => options.last_modified_time(time),
        None => options,
    }
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

/// Buffers the archive into chunks for the response body. Once aborted, it swallows writes.
struct ChannelWriter {
    tx: mpsc::Sender<Chunk>,
    buf: Vec<u8>,
    aborted: Arc<AtomicBool>,
}

impl ChannelWriter {
    fn new(tx: mpsc::Sender<Chunk>) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(CHUNK),
            aborted: Arc::new(AtomicBool::new(false)),
        }
    }

    fn send(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(mem::replace(&mut self.buf, Vec::with_capacity(CHUNK)));
        self.tx.blocking_send(Ok(chunk)).map_err(|_| {
            self.aborted.store(true, Ordering::Relaxed);
            io::Error::from(io::ErrorKind::BrokenPipe)
        })
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.aborted.load(Ordering::Relaxed) {
            return Ok(data.len());
        }
        let n = data.len().min(CHUNK - self.buf.len());
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() == CHUNK {
            self.send()?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.aborted.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.send()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer(capacity: usize) -> (ChannelWriter, mpsc::Receiver<Chunk>) {
        let (tx, rx) = mpsc::channel(capacity);
        (ChannelWriter::new(tx), rx)
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
        for short in 0..512 {
            let (tx, mut rx) = mpsc::channel(64);
            let path = RelPath::parse("big.bin").unwrap();
            stream_archive("media", &RelPath::root(), tx, |zip| {
                zip.start_file(path.as_str(), file_options(None))
                    .map_err(|err| Failure::at(&path, err))?;
                copy_data(zip, &path, FailsAfter(3 * CHUNK - short))
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
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let path = RelPath::parse("big.bin").unwrap();
        let sink = ChannelWriter::new(tx);
        let aborted = Arc::clone(&sink.aborted);
        let result = write_archive(sink, &RelPath::root(), |zip| {
            zip.start_file(path.as_str(), file_options(None))
                .map_err(|err| Failure::at(&path, err))?;
            copy_data(zip, &path, FailsAfter(3 * CHUNK))
        });
        assert!(result.is_err());
        assert!(aborted.load(Ordering::Relaxed));
    }

    #[test]
    fn writer_sends_full_chunks_then_the_rest_on_flush() {
        let (mut writer, mut rx) = writer(4);
        writer.write_all(&vec![7; CHUNK + 10]).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let sizes: Vec<usize> = std::iter::from_fn(|| rx.blocking_recv())
            .map(|chunk| chunk.unwrap().len())
            .collect();
        assert_eq!(sizes, [CHUNK, 10]);
    }

    #[test]
    fn writer_fails_once_the_receiver_is_gone() {
        let (mut writer, rx) = writer(1);
        drop(rx);
        let err = writer.write_all(&vec![0; CHUNK]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn entry_names_are_relative_to_the_zipped_folder() {
        let root = RelPath::parse("Docs").unwrap();
        let path = RelPath::parse("Docs/sub/b.txt").unwrap();
        assert_eq!(entry_name(&root, &path), "sub/b.txt");
        assert_eq!(entry_name(&RelPath::root(), &path), "Docs/sub/b.txt");
    }

    #[test]
    fn backslashes_in_entry_names_are_replaced() {
        let root = RelPath::root();
        let path = RelPath::parse("a\\..\\..\\x").unwrap();
        assert_eq!(entry_name(&root, &path), "a_.._.._x");
    }

    #[test]
    fn mtime_outside_zip_range_is_dropped() {
        assert!(zip_time(UNIX_EPOCH).is_none());
        let time = zip_time(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)).unwrap();
        assert_eq!((time.year(), time.month(), time.day()), (2023, 11, 14));
    }
}
