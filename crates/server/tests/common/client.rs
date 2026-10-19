use std::collections::HashMap;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use tower::ServiceExt;
use url::Url;

use super::fake_oidc::FakeOidc;

pub const COOKIE: &str = "explorer_session";

/// Carries the session cookie between requests the way a browser would.
#[derive(Default, Clone)]
pub struct Jar(pub Option<String>);

impl Jar {
    pub fn store(&mut self, res: &Response) {
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

    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// An app and a cookie jar, to send requests as one browser.
pub struct Client {
    pub app: Router,
    pub jar: Jar,
}

impl Client {
    pub fn new(app: Router) -> Self {
        Self {
            app,
            jar: Jar::default(),
        }
    }

    /// Runs the whole sign-in flow against `fake` with its default claims.
    pub async fn signed_in(app: Router, fake: &FakeOidc) -> Self {
        let mut client = Self::new(app);
        let res = client.get("/auth/login").await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let url = Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let code = fake.approve(&url).unwrap();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("code", &code)
            .append_pair("state", &params["state"])
            .finish();
        let res = client.get(&format!("/auth/callback?{query}")).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        client
    }

    pub async fn send(&mut self, mut req: Request<Body>) -> Response {
        if let Some(value) = self.jar.value() {
            req.headers_mut()
                .insert(header::COOKIE, format!("{COOKIE}={value}").parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        self.jar.store(&res);
        res
    }

    pub async fn get(&mut self, uri: &str) -> Response {
        self.send(super::get(uri)).await
    }
}

pub async fn json<T: serde::de::DeserializeOwned>(res: Response) -> T {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
