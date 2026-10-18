//! Container healthcheck client, so the runtime image needs no curl.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum HealthcheckError {
    #[error("request to {addr} failed: {source}")]
    Io { addr: SocketAddr, source: io::Error },
    #[error("request to {0} timed out")]
    Timeout(SocketAddr),
    #[error("unexpected response: {0}")]
    Status(String),
}

pub async fn run(addr: SocketAddr) -> Result<(), HealthcheckError> {
    let response = tokio::time::timeout(TIMEOUT, request(addr))
        .await
        .map_err(|_| HealthcheckError::Timeout(addr))?
        .map_err(|source| HealthcheckError::Io { addr, source })?;
    let status_line = response.lines().next().unwrap_or_default();
    let status = status_line.split_whitespace().nth(1);
    if status == Some("200") {
        Ok(())
    } else {
        Err(HealthcheckError::Status(status_line.to_owned()))
    }
}

async fn request(addr: SocketAddr) -> io::Result<String> {
    let mut stream = TcpStream::connect(addr).await?;
    let req = format!("GET /healthz HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.take(MAX_RESPONSE).read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
