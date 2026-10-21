//! Runs the fake provider standalone: `cargo run -p fake-oidc -- [ADDR]` (default 127.0.0.1:9000).
//! `/authorize` approves immediately and redirects back with a code.

use fake_oidc::{CLIENT_ID, CLIENT_SECRET, FakeOidc};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9000".to_owned());
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|err| panic!("cannot bind {addr}: {err}"));
    let fake = FakeOidc::start_on(listener);
    println!("issuer:        {}", fake.issuer());
    println!("client id:     {CLIENT_ID}");
    println!("client secret: {CLIENT_SECRET}");
    tokio::signal::ctrl_c().await.expect("wait for ctrl-c");
}
