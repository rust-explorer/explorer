use std::io::{self, Write};
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

pub(super) const CHUNK: usize = 64 * 1024;
pub(super) const ZIP_SEND_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) type Chunk = io::Result<Bytes>;

/// Fails with `TimedOut` when the channel stays full for `timeout`, so a stalled client cannot
/// hold the blocking thread forever. Runs on a blocking thread of the runtime.
pub(super) fn send_timeout(
    tx: &mpsc::Sender<Chunk>,
    chunk: Chunk,
    timeout: Duration,
) -> io::Result<()> {
    let sent = Handle::current().block_on(tokio::time::timeout(timeout, tx.send(chunk)));
    match sent {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(io::ErrorKind::BrokenPipe.into()),
        Err(_) => Err(io::ErrorKind::TimedOut.into()),
    }
}

/// Buffers the archive into chunks for the response body. Once aborted, it swallows writes.
pub(super) struct ChannelWriter {
    tx: mpsc::Sender<Chunk>,
    buf: Vec<u8>,
    pub(super) aborted: Arc<AtomicBool>,
    send_timeout: Duration,
}

impl ChannelWriter {
    pub(super) fn new(tx: mpsc::Sender<Chunk>) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(CHUNK),
            aborted: Arc::new(AtomicBool::new(false)),
            send_timeout: ZIP_SEND_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_send_timeout(mut self, timeout: Duration) -> Self {
        self.send_timeout = timeout;
        self
    }

    fn send(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(mem::replace(&mut self.buf, Vec::with_capacity(CHUNK)));
        send_timeout(&self.tx, Ok(chunk), self.send_timeout).inspect_err(|_| {
            self.aborted.store(true, Ordering::Relaxed);
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
    use std::time::Instant;

    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    fn writer(capacity: usize) -> (ChannelWriter, mpsc::Receiver<Chunk>) {
        let (tx, rx) = mpsc::channel(capacity);
        (ChannelWriter::new(tx), rx)
    }

    #[test]
    fn writer_sends_full_chunks_then_the_rest_on_flush() {
        let rt = runtime();
        let _entered = rt.enter();
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
        let rt = runtime();
        let _entered = rt.enter();
        let (mut writer, rx) = writer(1);
        drop(rx);
        let err = writer.write_all(&vec![0; CHUNK]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn stalled_client_times_out() {
        let rt = runtime();
        let _entered = rt.enter();
        let (tx, _rx) = mpsc::channel(1);
        let mut writer = ChannelWriter::new(tx).with_send_timeout(Duration::from_millis(50));
        writer.write_all(&vec![0; CHUNK]).unwrap();
        let started = Instant::now();
        let err = writer.write_all(&vec![0; CHUNK]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
