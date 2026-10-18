mod common;

use std::net::SocketAddr;

use explorer_server::healthcheck;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn healthcheck_succeeds_against_running_server() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::app_with_web_dir(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    healthcheck::run(addr).await.unwrap();
}

#[tokio::test]
async fn healthcheck_fails_when_nothing_listens() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);

    assert!(healthcheck::run(addr).await.is_err());
}

#[tokio::test]
async fn healthcheck_fails_on_non_200() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        }
    });

    assert!(healthcheck::run(addr).await.is_err());
}
