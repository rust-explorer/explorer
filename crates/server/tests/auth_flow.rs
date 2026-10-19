mod common;

use std::collections::HashMap;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use common::fake_oidc::{self, FakeOidc};
use explorer_server::app::router;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shared::api::{ErrorBody, LogoutResponse, Me};
use tower::ServiceExt;
use url::Url;

const COOKIE: &str = "explorer_session";

struct Harness {
    app: Router,
    fake: FakeOidc,
    _dir: tempfile::TempDir,
}

/// Carries the session cookie between requests the way a browser would.
#[derive(Default, Clone)]
struct Jar(Option<String>);

impl Jar {
    fn store(&mut self, res: &Response) {
        for value in res.headers().get_all(header::SET_COOKIE) {
            let value = value.to_str().unwrap();
            let Some(pair) = value.split(';').next() else {
                continue;
            };
            let Some((name, cookie)) = pair.split_once('=') else {
                continue;
            };
            if name != COOKIE {
                continue;
            }
            let removed = value.split("; ").any(|a| a == "Max-Age=0");
            self.0 = (!removed).then(|| cookie.to_owned());
        }
    }

    fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (state, fake) = common::state_with_oidc(dir.path()).await;
        let app = router(state).await.unwrap();
        Self {
            app,
            fake,
            _dir: dir,
        }
    }

    async fn send(&self, jar: &mut Jar, mut req: Request<Body>) -> Response {
        if let Some(value) = jar.value() {
            req.headers_mut()
                .insert(header::COOKIE, format!("{COOKIE}={value}").parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        jar.store(&res);
        res
    }

    async fn get(&self, jar: &mut Jar, uri: &str) -> Response {
        self.send(jar, common::get(uri)).await
    }

    /// Starts a login and returns the provider URL the browser is sent to.
    async fn login(&self, jar: &mut Jar, return_to: Option<&str>) -> Url {
        let uri = match return_to {
            Some(r) => format!(
                "/auth/login?{}",
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("return_to", r)
                    .finish()
            ),
            None => "/auth/login".to_owned(),
        };
        let res = self.get(jar, &uri).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let url = Url::parse(location(&res)).unwrap();
        assert!(url.as_str().starts_with(&self.fake.issuer()), "{url}");
        url
    }

    async fn callback(&self, jar: &mut Jar, params: &[(&str, &str)]) -> Response {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        self.get(jar, &format!("/auth/callback?{query}")).await
    }

    /// Runs the whole flow with the given ID token claim overrides; returns the callback response
    /// and the parameters it was called with.
    async fn sign_in(&self, jar: &mut Jar, claims: Value) -> (Response, Vec<(String, String)>) {
        let url = self.login(jar, None).await;
        self.fake.next_claims(claims);
        let code = self.fake.approve(&url).unwrap();
        let params = vec![
            ("code".to_owned(), code),
            ("state".to_owned(), state_of(&url)),
        ];
        let res = self.callback_owned(jar, &params).await;
        (res, params)
    }

    async fn callback_owned(&self, jar: &mut Jar, params: &[(String, String)]) -> Response {
        let borrowed: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.callback(jar, &borrowed).await
    }

    async fn me_status(&self, jar: &mut Jar) -> StatusCode {
        self.get(jar, "/api/me").await.status()
    }

    async fn logout(&self, jar: &mut Jar, origin: Option<&str>) -> Response {
        let mut builder = Request::post("/auth/logout");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        self.send(jar, builder.body(Body::empty()).unwrap()).await
    }
}

fn location(res: &Response) -> &str {
    res.headers()[header::LOCATION].to_str().unwrap()
}

fn state_of(url: &Url) -> String {
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    query["state"].clone()
}

async fn json<T: serde::de::DeserializeOwned>(res: Response) -> T {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn assert_signin_error(res: &Response, reason: &str) {
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(res), format!("/signin-error?reason={reason}"));
}

#[tokio::test]
async fn me_requires_login() {
    let h = Harness::new().await;
    let res = h.get(&mut Jar::default(), "/api/me").await;

    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "unauthenticated");
}

#[tokio::test]
async fn full_login_flow_sets_session_and_redirects_to_return_to() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let url = h.login(&mut jar, Some("/docs/a?b=1")).await;
    h.fake.next_claims(json!({ "name": "Ada Lovelace" }));
    let code = h.fake.approve(&url).unwrap();
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state_of(&url))])
        .await;

    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/docs/a?b=1");
    let res = h.get(&mut jar, "/api/me").await;
    assert_eq!(res.status(), StatusCode::OK);
    let me: Me = json(res).await;
    assert_eq!(
        me,
        Me {
            sub: fake_oidc::SUBJECT.to_owned(),
            name: "Ada Lovelace".to_owned(),
            email: Some("user@example.com".to_owned()),
        }
    );
}

#[tokio::test]
async fn open_redirect_return_to_falls_back_to_root() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let url = h.login(&mut jar, Some("//evil.example")).await;
    let code = h.fake.approve(&url).unwrap();
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state_of(&url))])
        .await;

    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/");
}

#[tokio::test]
async fn session_id_rotates_on_login() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let url = h.login(&mut jar, None).await;
    let before = jar.value().unwrap().to_owned();
    let code = h.fake.approve(&url).unwrap();
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state_of(&url))])
        .await;

    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let after = jar.value().unwrap().to_owned();
    assert_ne!(before, after);
    assert_eq!(
        h.me_status(&mut Jar(Some(before))).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn user_outside_allowed_group_is_denied() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let (res, _) = h
        .sign_in(&mut jar, json!({ "groups": ["other-group"] }))
        .await;

    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&res), "/access-denied");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn denied_sign_in_drops_an_existing_session_user() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    h.sign_in(&mut jar, json!({})).await;
    assert_eq!(h.me_status(&mut jar).await, StatusCode::OK);

    let (res, _) = h.sign_in(&mut jar, json!({ "groups": [] })).await;

    assert_eq!(location(&res), "/access-denied");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_groups_claim_reports_reason() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let (res, _) = h.sign_in(&mut jar, json!({ "groups": null })).await;

    assert_signin_error(&res, "missing_groups_claim");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn groups_claim_as_string_reports_missing_groups_claim() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let (res, _) = h
        .sign_in(&mut jar, json!({ "groups": common::ALLOWED_GROUP }))
        .await;

    assert_signin_error(&res, "missing_groups_claim");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn state_mismatch_is_rejected() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let url = h.login(&mut jar, None).await;
    let code = h.fake.approve(&url).unwrap();
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", "forged")])
        .await;

    assert_signin_error(&res, "state_mismatch");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);

    // The pending login was consumed by the failed attempt.
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state_of(&url))])
        .await;
    assert_signin_error(&res, "state_mismatch");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn callback_without_pending_login_is_rejected() {
    let h = Harness::new().await;
    let mut jar = Jar::default();

    let res = h.callback(&mut jar, &[("code", "c"), ("state", "s")]).await;

    assert_signin_error(&res, "state_mismatch");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn callback_without_code_or_state_is_state_mismatch() {
    let h = Harness::new().await;
    for params in [vec![], vec![("code", "c")], vec![("state", "s")]] {
        let mut jar = Jar::default();
        let url = h.login(&mut jar, None).await;
        let state = state_of(&url);
        let params: Vec<(&str, &str)> = params
            .into_iter()
            .map(|(k, v)| (k, if k == "state" { state.as_str() } else { v }))
            .collect();

        let res = h.callback(&mut jar, &params).await;

        assert_signin_error(&res, "state_mismatch");
        assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn callback_replay_is_rejected() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    let url = h.login(&mut jar, None).await;
    let pre_login = jar.clone();
    let code = h.fake.approve(&url).unwrap();
    let params = [
        ("code".to_owned(), code),
        ("state".to_owned(), state_of(&url)),
    ];
    let res = h.callback_owned(&mut jar, &params).await;
    assert_eq!(location(&res), "/");

    // Replayed from the session that started the login (now rotated away), and from none.
    for mut replay in [pre_login, Jar::default()] {
        let res = h.callback_owned(&mut replay, &params).await;
        assert_signin_error(&res, "state_mismatch");
        assert_eq!(h.me_status(&mut replay).await, StatusCode::UNAUTHORIZED);
    }

    // Replayed from the signed-in session: rejected, the existing session is left alone.
    let res = h.callback_owned(&mut jar, &params).await;
    assert_signin_error(&res, "state_mismatch");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::OK);
}

#[tokio::test]
async fn expired_or_wrong_audience_token_is_invalid_token() {
    let h = Harness::new().await;
    for claims in [
        json!({ "exp": 1_000_000_000, "iat": 999_999_000 }),
        json!({ "aud": "someone-else" }),
    ] {
        let mut jar = Jar::default();
        let (res, _) = h.sign_in(&mut jar, claims).await;

        assert_signin_error(&res, "invalid_token");
        assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn token_signed_with_an_unpublished_key_is_invalid_token() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    let url = h.login(&mut jar, None).await;
    h.fake.sign_next_with_rogue_key();
    let code = h.fake.approve(&url).unwrap();

    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state_of(&url))])
        .await;

    assert_signin_error(&res, "invalid_token");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_code_is_exchange_failed() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    let url = h.login(&mut jar, None).await;

    let res = h
        .callback(&mut jar, &[("code", "unknown"), ("state", &state_of(&url))])
        .await;

    assert_signin_error(&res, "exchange_failed");
}

#[tokio::test]
async fn provider_error_param_reports_provider_error() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    let url = h.login(&mut jar, None).await;
    let state = state_of(&url);

    let res = h
        .callback(&mut jar, &[("error", "access_denied"), ("state", &state)])
        .await;

    assert_signin_error(&res, "provider_error");
    // The pending login is consumed even though the provider gave up.
    let code = h.fake.approve(&url).unwrap();
    let res = h
        .callback(&mut jar, &[("code", &code), ("state", &state)])
        .await;
    assert_signin_error(&res, "state_mismatch");
}

#[tokio::test]
async fn logout_clears_session_and_returns_end_session_url() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    h.sign_in(&mut jar, json!({})).await;
    let signed_in = jar.clone();

    let res = h.logout(&mut jar, Some(common::PUBLIC_URL)).await;

    assert_eq!(res.status(), StatusCode::OK);
    let body: LogoutResponse = json(res).await;
    let url = Url::parse(&body.redirect_to).unwrap();
    assert_eq!(
        url.as_str().split('?').next().unwrap(),
        format!("{}/end-session", h.fake.issuer())
    );
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query["id_token_hint"].split('.').count(), 3);
    assert_eq!(query["post_logout_redirect_uri"], common::PUBLIC_URL);
    assert_eq!(h.me_status(&mut jar).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.me_status(&mut signed_in.clone()).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn denied_user_can_still_end_the_provider_session() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    h.sign_in(&mut jar, json!({ "groups": ["other-group"] }))
        .await;

    let res = h.logout(&mut jar, Some(common::PUBLIC_URL)).await;

    assert_eq!(res.status(), StatusCode::OK);
    let body: LogoutResponse = json(res).await;
    assert!(
        body.redirect_to
            .starts_with(&format!("{}/end-session?", h.fake.issuer())),
        "{}",
        body.redirect_to
    );
}

#[tokio::test]
async fn logout_without_session_redirects_home() {
    let h = Harness::new().await;
    let res = h
        .logout(&mut Jar::default(), Some(common::PUBLIC_URL))
        .await;

    assert_eq!(res.status(), StatusCode::OK);
    let body: LogoutResponse = json(res).await;
    assert_eq!(body.redirect_to, "/");
}

#[tokio::test]
async fn logout_without_origin_is_rejected() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    h.sign_in(&mut jar, json!({})).await;

    let res = h.logout(&mut jar, None).await;

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "csrf_rejected");
    assert_eq!(h.me_status(&mut jar).await, StatusCode::OK);
}

#[tokio::test]
async fn unknown_auth_path_is_json_404() {
    let h = Harness::new().await;
    let res = h.get(&mut Jar::default(), "/auth/nope").await;

    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body: ErrorBody = json(res).await;
    assert_eq!(body.code, "not_found");
}

#[tokio::test]
async fn failed_callback_consumes_pending_login_even_if_cookie_is_kept() {
    let h = Harness::new().await;
    let mut jar = Jar::default();
    let url = h.login(&mut jar, None).await;
    let state = state_of(&url);
    let kept = jar.clone();

    let res = h
        .callback(&mut jar, &[("code", "c"), ("state", "forged")])
        .await;
    assert_signin_error(&res, "state_mismatch");

    // A client ignoring the cookie removal must not get a second try at the pending login.
    let code = h.fake.approve(&url).unwrap();
    let mut replay = kept;
    let res = h
        .callback(&mut replay, &[("code", &code), ("state", &state)])
        .await;
    assert_signin_error(&res, "state_mismatch");
    assert_eq!(h.me_status(&mut replay).await, StatusCode::UNAUTHORIZED);
}
