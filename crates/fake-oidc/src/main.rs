//! Runs the fake provider standalone for manual checks:
//! `cargo run -p fake-oidc -- [ADDR]` (default 127.0.0.1:8081).
//! `/authorize` approves immediately, signing in a demo user who belongs to `explorer-users`.

use fake_oidc::{CLIENT_ID, CLIENT_SECRET, FakeOidc};
use serde_json::json;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8081".to_owned());
    let listener = TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|err| panic!("cannot bind {addr}: {err}"));
    let fake = FakeOidc::start_on(listener);
    fake.claims(json!({
        "name": "Ada Lovelace",
        "preferred_username": "ada",
        "email": "ada@example.com",
        "groups": ["explorer-users"],
    }));
    println!("issuer:        {}", fake.issuer());
    println!("client id:     {CLIENT_ID}");
    println!("client secret: {CLIENT_SECRET}");
    println!("demo user:     Ada Lovelace <ada@example.com>, group explorer-users");
    tokio::signal::ctrl_c().await.expect("wait for ctrl-c");
}
