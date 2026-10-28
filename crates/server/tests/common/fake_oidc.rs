//! In-process `OpenID Connect` provider for the integration tests.
#![allow(clippy::missing_panics_doc)]

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use rand::RngCore;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

pub const CLIENT_ID: &str = "explorer";
pub const CLIENT_SECRET: &str = "fake-secret";
pub const SUBJECT: &str = "user-1";

const KEY_ID: &str = "fake-oidc-key";
const TOKEN_TTL_SECS: u64 = 300;

static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();

/// Returns the process-wide signing key, generating it on first use.
pub fn generate_key() -> &'static RsaPrivateKey {
    KEY.get_or_init(|| {
        RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("generate RSA test key")
    })
}

/// A running fake provider; the server stops when this is dropped.
pub struct FakeOidc {
    issuer: String,
    shared: Arc<Shared>,
    server: JoinHandle<()>,
}

struct Shared {
    issuer: String,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    authorizations: HashMap<String, Authorization>,
    access_tokens: HashMap<String, String>,
    claims: Option<Value>,
    next_claims: Option<Value>,
    next_userinfo: Option<Value>,
    sign_next_with_rogue_key: bool,
    discovery_outage: VecDeque<Outage>,
}

/// What a reverse proxy typically answers while the provider behind it restarts.
#[derive(Debug, Clone, Copy)]
pub enum Outage {
    NotFound,
    MaintenancePage,
    BadGateway,
}

impl IntoResponse for Outage {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "404 page not found").into_response(),
            Self::MaintenancePage => {
                Html("<html><body>Service is restarting</body></html>").into_response()
            }
            Self::BadGateway => (StatusCode::BAD_GATEWAY, "502 Bad Gateway").into_response(),
        }
    }
}

struct Authorization {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    nonce: Option<String>,
}

#[derive(Debug)]
pub enum AuthorizeError {
    MissingParameter(&'static str),
    UnsupportedChallengeMethod,
}

impl FakeOidc {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake OIDC provider");
        Self::start_on(listener)
    }

    pub fn start_on(listener: TcpListener) -> Self {
        let addr: SocketAddr = listener.local_addr().expect("listener address");
        let issuer = format!("http://{addr}");
        generate_key();
        let shared = Arc::new(Shared {
            issuer: issuer.clone(),
            inner: Mutex::new(Inner::default()),
        });
        let app = router(Arc::clone(&shared));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("fake OIDC server");
        });
        Self {
            issuer,
            shared,
            server,
        }
    }

    /// The base URL without a trailing slash, as advertised in discovery.
    #[must_use]
    pub fn issuer(&self) -> String {
        self.issuer.clone()
    }

    /// Overrides claims of every later ID token. A `null` value removes the claim.
    pub fn claims(&self, claims: Value) {
        self.shared.lock().claims = Some(claims);
    }

    /// Overrides claims of the next ID token. A `null` value removes the claim.
    pub fn next_claims(&self, claims: Value) {
        self.shared.lock().next_claims = Some(claims);
    }

    /// Signs the next ID token with a throwaway key that the JWKS does not publish.
    pub fn sign_next_with_rogue_key(&self) {
        self.shared.lock().sign_next_with_rogue_key = true;
    }

    /// Claims added to the next userinfo response (which always carries `sub`).
    /// A `null` value removes the claim.
    pub fn next_userinfo(&self, claims: Value) {
        self.shared.lock().next_userinfo = Some(claims);
    }

    /// Answers the next discovery requests with these failures, one per request, in order.
    pub fn discovery_outage(&self, outages: impl IntoIterator<Item = Outage>) {
        self.shared.lock().discovery_outage.extend(outages);
    }

    /// Records what the browser would send to `/authorize` and returns the authorization code.
    pub fn approve(&self, authorize_url: &Url) -> Result<String, AuthorizeError> {
        let params: HashMap<String, String> = authorize_url.query_pairs().into_owned().collect();
        self.shared.authorize(&params)
    }
}

impl Drop for FakeOidc {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn authorize(&self, params: &HashMap<String, String>) -> Result<String, AuthorizeError> {
        let param = |name: &'static str| {
            params
                .get(name)
                .cloned()
                .ok_or(AuthorizeError::MissingParameter(name))
        };
        if param("code_challenge_method")? != "S256" {
            return Err(AuthorizeError::UnsupportedChallengeMethod);
        }
        let authorization = Authorization {
            client_id: param("client_id")?,
            redirect_uri: param("redirect_uri")?,
            code_challenge: param("code_challenge")?,
            nonce: params.get("nonce").cloned(),
        };
        let code = random_token();
        self.lock()
            .authorizations
            .insert(code.clone(), authorization);
        Ok(code)
    }

    fn id_token(&self, authorization: &Authorization) -> String {
        let now = unix_now();
        let mut claims = json!({
            "iss": self.issuer,
            "sub": SUBJECT,
            "aud": authorization.client_id,
            "iat": now,
            "exp": now + TOKEN_TTL_SECS,
            "name": "Test User",
            "preferred_username": "test",
            "email": "user@example.com",
            "groups": ["explorer-users"],
        });
        if let Some(nonce) = &authorization.nonce {
            claims["nonce"] = json!(nonce);
        }
        let (persistent, next, rogue) = {
            let mut inner = self.lock();
            (
                inner.claims.clone(),
                inner.next_claims.take(),
                std::mem::take(&mut inner.sign_next_with_rogue_key),
            )
        };
        for overrides in [persistent, next].into_iter().flatten() {
            merge(&mut claims, overrides);
        }
        if rogue {
            let key =
                RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("generate rogue key");
            return sign(&claims, &key);
        }
        sign(&claims, generate_key())
    }
}

fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .route("/end-session", get(end_session))
        .with_state(shared)
}

async fn discovery(State(shared): State<Arc<Shared>>) -> Response {
    if let Some(outage) = shared.lock().discovery_outage.pop_front() {
        return outage.into_response();
    }
    let issuer = &shared.issuer;
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "end_session_endpoint": format!("{issuer}/end-session"),
        "jwks_uri": format!("{issuer}/jwks"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["openid", "profile", "email", "groups"],
        "claims_supported": ["sub", "name", "preferred_username", "email", "groups"],
    }))
    .into_response()
}

async fn jwks() -> Json<Value> {
    let public = RsaPublicKey::from(generate_key());
    Json(json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": KEY_ID,
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }]
    }))
}

async fn authorize(
    State(shared): State<Arc<Shared>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let code = match shared.authorize(&params) {
        Ok(code) => code,
        Err(err) => return (StatusCode::BAD_REQUEST, format!("{err:?}")).into_response(),
    };
    let Ok(mut redirect) = Url::parse(&params["redirect_uri"]) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    };
    redirect.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = params.get("state") {
        redirect.query_pairs_mut().append_pair("state", state);
    }
    Redirect::to(redirect.as_str()).into_response()
}

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
    code: String,
    redirect_uri: String,
    code_verifier: String,
    client_id: Option<String>,
    client_secret: Option<String>,
}

async fn token(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    if !client_authenticated(&headers, &form) {
        return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    if form.grant_type != "authorization_code" {
        return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    let Some(authorization) = shared.lock().authorizations.remove(&form.code) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form.code_verifier.as_bytes()));
    if authorization.redirect_uri != form.redirect_uri
        || authorization.code_challenge != challenge
        || authorization.client_id != CLIENT_ID
    {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let id_token = shared.id_token(&authorization);
    let access_token = random_token();
    shared
        .lock()
        .access_tokens
        .insert(access_token.clone(), SUBJECT.to_owned());
    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": TOKEN_TTL_SECS,
        "id_token": id_token,
    }))
    .into_response()
}

fn client_authenticated(headers: &HeaderMap, form: &TokenForm) -> bool {
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| STANDARD.decode(v).ok());
    match basic {
        Some(decoded) => decoded == format!("{CLIENT_ID}:{CLIENT_SECRET}").as_bytes(),
        None => {
            form.client_id.as_deref() == Some(CLIENT_ID)
                && form.client_secret.as_deref() == Some(CLIENT_SECRET)
        }
    }
}

async fn userinfo(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let mut inner = shared.lock();
    let Some(sub) = token.and_then(|t| inner.access_tokens.get(t)).cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut claims = json!({ "sub": sub });
    if let Some(overrides) = inner.next_userinfo.take() {
        merge(&mut claims, overrides);
    }
    Json(claims).into_response()
}

#[derive(Deserialize)]
struct EndSession {
    post_logout_redirect_uri: Option<String>,
}

async fn end_session(Query(query): Query<EndSession>) -> Response {
    match query.post_logout_redirect_uri {
        Some(uri) => Redirect::to(&uri).into_response(),
        None => "signed out".into_response(),
    }
}

fn oauth_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

fn merge(claims: &mut Value, overrides: Value) {
    let (Value::Object(target), Value::Object(overrides)) = (claims, overrides) else {
        panic!("claim overrides must be a JSON object");
    };
    for (key, value) in overrides {
        if value.is_null() {
            target.remove(&key);
        } else {
            target.insert(key, value);
        }
    }
}

fn sign(claims: &Value, key: &RsaPrivateKey) -> String {
    let header = json!({ "alg": "RS256", "typ": "JWT", "kid": KEY_ID });
    let encode = |v: &Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).expect("serialize JWT"));
    let signing_input = format!("{}.{}", encode(&header), encode(claims));
    let signature = SigningKey::<Sha256>::new(key.clone()).sign(signing_input.as_bytes());
    format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    )
}

fn random_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overrides_and_removes_claims() {
        let mut claims = json!({ "a": 1, "b": 2 });
        merge(&mut claims, json!({ "a": 3, "b": null, "c": 4 }));
        assert_eq!(claims, json!({ "a": 3, "c": 4 }));
    }

    #[test]
    fn persistent_claims_apply_to_every_token_before_one_shot_overrides() {
        let shared = Shared {
            issuer: "http://127.0.0.1:1".to_owned(),
            inner: Mutex::new(Inner::default()),
        };
        shared.lock().claims = Some(json!({ "name": "Ada Lovelace", "email": null }));
        shared.lock().next_claims = Some(json!({ "name": "Once" }));
        let authorization = Authorization {
            client_id: CLIENT_ID.to_owned(),
            redirect_uri: String::new(),
            code_challenge: String::new(),
            nonce: None,
        };
        let payload = |token: &str| -> Value {
            let part = token.split('.').nth(1).unwrap();
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
        };

        let first = payload(&shared.id_token(&authorization));
        let second = payload(&shared.id_token(&authorization));

        assert_eq!(first["name"], "Once");
        assert_eq!(second["name"], "Ada Lovelace");
        assert!(second.get("email").is_none());
        assert_eq!(second["groups"], json!(["explorer-users"]));
    }

    #[test]
    fn authorize_requires_s256() {
        let shared = Shared {
            issuer: "http://127.0.0.1:1".to_owned(),
            inner: Mutex::new(Inner::default()),
        };
        let params: HashMap<String, String> = [
            ("client_id", CLIENT_ID),
            ("redirect_uri", "https://files.example.com/auth/callback"),
            ("code_challenge", "abc"),
            ("code_challenge_method", "plain"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert!(matches!(
            shared.authorize(&params),
            Err(AuthorizeError::UnsupportedChallengeMethod)
        ));
    }
}
