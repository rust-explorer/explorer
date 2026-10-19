mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::fake_oidc::{self, FakeOidc, Outage};
use explorer_server::oidc::{Oidc, SigninError, VerifiedIdentity};
use serde_json::{Value, json};
use tokio::net::TcpListener;

#[tokio::test]
async fn discovers_fake_provider() {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &fake.issuer());

    Oidc::discover(&config.oidc, &config.public_url)
        .await
        .unwrap();
}

#[tokio::test]
async fn issuer_mismatch_is_an_error() {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &format!("{}/", fake.issuer()));

    let Err(err) = Oidc::discover(&config.oidc, &config.public_url).await else {
        panic!("a trailing-slash mismatch must be rejected");
    };
    assert!(err.to_string().contains("unexpected issuer"), "{err}");
}

#[tokio::test]
async fn issuer_mismatch_is_not_retried() {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &format!("{}/", fake.issuer()));

    let started = Instant::now();
    let result =
        Oidc::discover_with_retry(&config.oidc, &config.public_url, 5, Duration::from_secs(1))
            .await;

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn retry_succeeds_when_provider_comes_up_late() {
    fake_oidc::generate_key();
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let config = common::config("/unused", &format!("http://{addr}"));
    let late = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        FakeOidc::start_on(TcpListener::bind(addr).await.unwrap())
    });

    let started = Instant::now();
    let result = Oidc::discover_with_retry(
        &config.oidc,
        &config.public_url,
        8,
        Duration::from_millis(300),
    )
    .await;

    let _fake = late.await.unwrap();
    result.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
}

#[tokio::test]
async fn retry_survives_proxy_errors_while_provider_restarts() {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &fake.issuer());
    let outages = || {
        [
            Outage::NotFound,
            Outage::MaintenancePage,
            Outage::BadGateway,
        ]
    };

    fake.discovery_outage(outages());
    let result = Oidc::discover_with_retry(
        &config.oidc,
        &config.public_url,
        3,
        Duration::from_millis(10),
    )
    .await;
    assert!(
        result.is_err(),
        "three failures must exhaust three attempts"
    );

    fake.discovery_outage(outages());
    Oidc::discover_with_retry(
        &config.oidc,
        &config.public_url,
        4,
        Duration::from_millis(10),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn retry_gives_up_with_clear_error() {
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let issuer = format!("http://{addr}");
    let config = common::config("/unused", &issuer);

    let Err(err) = Oidc::discover_with_retry(
        &config.oidc,
        &config.public_url,
        2,
        Duration::from_millis(10),
    )
    .await
    else {
        panic!("discovery should fail with nothing listening");
    };

    assert!(err.to_string().contains(&issuer), "{err}");
}

#[tokio::test]
async fn authorize_url_contains_pkce_state_nonce_and_scopes() {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &fake.issuer());
    let oidc = Oidc::discover(&config.oidc, &config.public_url)
        .await
        .unwrap();

    let (url, pending) = oidc.authorize_url();

    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(!query["code_challenge"].is_empty());
    assert_eq!(query["state"], pending.csrf_state);
    assert_eq!(query["nonce"], pending.nonce);
    assert_eq!(query["scope"], "openid profile email groups");
    assert_eq!(
        query["redirect_uri"],
        "https://files.example.com/auth/callback"
    );
    assert_eq!(query["response_type"], "code");
    assert!(!query.contains_key("response_mode"));
}

async fn discovered() -> (Oidc, FakeOidc) {
    let fake = FakeOidc::start().await;
    let config = common::config("/unused", &fake.issuer());
    let oidc = Oidc::discover(&config.oidc, &config.public_url)
        .await
        .unwrap();
    (oidc, fake)
}

async fn sign_in(claims: Value, userinfo: Value) -> Result<VerifiedIdentity, SigninError> {
    let (oidc, fake) = discovered().await;
    fake.next_claims(claims);
    fake.next_userinfo(userinfo);
    let (url, pending) = oidc.authorize_url();
    let code = fake.approve(&url).unwrap();
    oidc.exchange(code, pending).await
}

#[tokio::test]
async fn exchange_returns_verified_identity() {
    let identity = sign_in(json!({}), json!({})).await.unwrap();

    assert_eq!(identity.sub, fake_oidc::SUBJECT);
    assert_eq!(identity.name, "Test User");
    assert_eq!(identity.email.as_deref(), Some("user@example.com"));
    assert_eq!(identity.groups, ["explorer-users"]);
    assert_eq!(identity.id_token.split('.').count(), 3);
}

#[tokio::test]
async fn name_falls_back_to_preferred_username_then_sub() {
    let identity = sign_in(json!({ "name": null }), json!({})).await.unwrap();
    assert_eq!(identity.name, "test");

    let identity = sign_in(
        json!({ "name": null, "preferred_username": null }),
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(identity.name, fake_oidc::SUBJECT);
}

#[tokio::test]
async fn empty_name_falls_back_to_preferred_username() {
    let identity = sign_in(json!({ "name": "" }), json!({})).await.unwrap();
    assert_eq!(identity.name, "test");
}

#[tokio::test]
async fn groups_fall_back_to_userinfo() {
    let identity = sign_in(
        json!({ "groups": null }),
        json!({ "groups": ["from-userinfo"] }),
    )
    .await
    .unwrap();

    assert_eq!(identity.groups, ["from-userinfo"]);
}

#[tokio::test]
async fn missing_groups_everywhere_is_rejected() {
    let err = sign_in(json!({ "groups": null }), json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, SigninError::MissingGroupsClaim), "{err}");
}

#[tokio::test]
async fn groups_that_are_not_strings_are_rejected() {
    let err = sign_in(json!({ "groups": "explorer-users" }), json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, SigninError::MissingGroupsClaim), "{err}");

    let err = sign_in(json!({ "groups": [1] }), json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, SigninError::MissingGroupsClaim), "{err}");
}

#[tokio::test]
async fn invalid_tokens_are_rejected() {
    let expired = json!({ "exp": 1_000_000_000, "iat": 999_999_000 });
    for claims in [
        json!({ "nonce": "replayed" }),
        json!({ "aud": "someone-else" }),
        json!({ "iss": "https://evil.example.com" }),
        expired,
    ] {
        let err = sign_in(claims.clone(), json!({})).await.unwrap_err();
        assert!(
            matches!(err, SigninError::InvalidToken(_)),
            "{claims}: {err}"
        );
    }
}

#[tokio::test]
async fn wrong_pkce_verifier_fails_the_exchange() {
    let (oidc, fake) = discovered().await;
    let (url, mut pending) = oidc.authorize_url();
    let code = fake.approve(&url).unwrap();
    pending.pkce_verifier = oidc.authorize_url().1.pkce_verifier;

    let err = oidc.exchange(code, pending).await.unwrap_err();
    assert!(matches!(err, SigninError::ExchangeFailed(_)), "{err}");
}

#[tokio::test]
async fn codes_are_single_use() {
    let (oidc, fake) = discovered().await;
    let (url, pending) = oidc.authorize_url();
    let code = fake.approve(&url).unwrap();
    oidc.exchange(code.clone(), pending.clone()).await.unwrap();

    let err = oidc.exchange(code, pending).await.unwrap_err();
    assert!(matches!(err, SigninError::ExchangeFailed(_)), "{err}");
}

#[tokio::test]
async fn end_session_url_carries_hint_and_redirect() {
    let (oidc, fake) = discovered().await;

    let url = oidc
        .end_session_url("the.id.token", "https://files.example.com")
        .unwrap();

    assert!(
        url.as_str()
            .starts_with(&format!("{}/end-session?", fake.issuer()))
    );
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query["id_token_hint"], "the.id.token");
    assert_eq!(
        query["post_logout_redirect_uri"],
        "https://files.example.com"
    );
    assert_eq!(query["client_id"], fake_oidc::CLIENT_ID);
}
