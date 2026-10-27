use serde::Serialize;
use serde::de::DeserializeOwned;
use shared::api::auth::LogoutResponse;
use shared::api::error::ErrorBody;

mod urls;

pub use urls::{
    FAVORITES_URL, download_url, encode_component, entries_url, favorite_url, folders_url,
    rename_url, upload_url, zip_selection_url, zip_url,
};
#[cfg_attr(not(target_arch = "wasm32"), allow(unused_imports))]
pub use urls::{TASKS_URL, task_url};

const UNAUTHENTICATED: &str = "unauthenticated";

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub enum ApiFailure {
    Network,
    /// An error status whose body isn't an `ErrorBody`.
    Http(u16),
    Server(ErrorBody),
}

impl ApiFailure {
    /// True while the browser is already on its way to the sign-in page.
    pub fn is_unauthenticated(&self) -> bool {
        matches!(self, Self::Server(body) if body.code == UNAUTHENTICATED)
    }
}

/// The sign-in URL that brings the user back to `path_and_query` afterwards.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn login_url(path_and_query: &str) -> String {
    format!("/auth/login?return_to={}", encode_component(path_and_query))
}

/// Ends the session here and at the provider when it supports it.
pub async fn sign_out() -> Result<(), ApiFailure> {
    let response: LogoutResponse = post_json("/auth/logout").await?;
    navigate(&response.redirect_to);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiFailure> {
    send(gloo_net::http::Request::get(path)).await
}

#[cfg(target_arch = "wasm32")]
pub async fn post_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiFailure> {
    send(gloo_net::http::Request::post(path)).await
}

/// POSTs `body` as JSON; an empty success response (204) reads as `null`, so `()` works.
#[cfg(target_arch = "wasm32")]
pub async fn post_json_body<B: Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiFailure> {
    send_json_body(gloo_net::http::Request::post(path), body).await
}

/// Like [`post_json_body`], with PUT.
#[cfg(target_arch = "wasm32")]
pub async fn put_json_body<B: Serialize, T: DeserializeOwned>(
    path: &str,
    body: &B,
) -> Result<T, ApiFailure> {
    send_json_body(gloo_net::http::Request::put(path), body).await
}

/// DELETEs `path`; an empty success response (204) reads as `null`, so `()` works.
#[cfg(target_arch = "wasm32")]
pub async fn delete_json<T: DeserializeOwned>(path: &str) -> Result<T, ApiFailure> {
    let request = gloo_net::http::Request::delete(path)
        .credentials(web_sys::RequestCredentials::SameOrigin)
        .header("accept", "application/json");
    read_json(request.send().await.map_err(|_| ApiFailure::Network)?).await
}

#[cfg(target_arch = "wasm32")]
async fn send_json_body<B: Serialize, T: DeserializeOwned>(
    request: gloo_net::http::RequestBuilder,
    body: &B,
) -> Result<T, ApiFailure> {
    let request = request
        .credentials(web_sys::RequestCredentials::SameOrigin)
        .header("accept", "application/json")
        .json(body)
        .map_err(|_| ApiFailure::Network)?;
    read_json(request.send().await.map_err(|_| ApiFailure::Network)?).await
}

#[cfg(target_arch = "wasm32")]
async fn read_json<T: DeserializeOwned>(
    response: gloo_net::http::Response,
) -> Result<T, ApiFailure> {
    let text = response.text().await.unwrap_or_default();
    if !response.ok() {
        return Err(failure(response.status(), &text));
    }
    let text = if text.is_empty() { "null" } else { &text };
    serde_json::from_str(text).map_err(|_| ApiFailure::Network)
}

#[cfg(target_arch = "wasm32")]
async fn send<T: DeserializeOwned>(
    request: gloo_net::http::RequestBuilder,
) -> Result<T, ApiFailure> {
    let response = request
        .credentials(web_sys::RequestCredentials::SameOrigin)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| ApiFailure::Network)?;
    if response.ok() {
        return response.json().await.map_err(|_| ApiFailure::Network);
    }
    let body = response.text().await.unwrap_or_default();
    Err(failure(response.status(), &body))
}

/// The failure an error response stands for; a 401 also sends the browser to sign-in.
#[cfg(target_arch = "wasm32")]
pub fn failure(status: u16, body: &str) -> ApiFailure {
    if status == 401 {
        navigate(&login_url(&current_path_and_query()));
        return ApiFailure::Server(ErrorBody {
            code: UNAUTHENTICATED.to_owned(),
            message: String::new(),
            details: None,
            request_id: None,
        });
    }
    serde_json::from_str(body).map_or(ApiFailure::Http(status), ApiFailure::Server)
}

#[cfg(target_arch = "wasm32")]
fn current_path_and_query() -> String {
    web_sys::window()
        .map(|window| window.location())
        .and_then(|location| Some(location.pathname().ok()? + &location.search().ok()?))
        .unwrap_or_else(|| "/".to_owned())
}

/// Full page navigation, for server routes that are not part of the SPA.
#[cfg(target_arch = "wasm32")]
pub fn navigate(url: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().assign(url);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get_json<T: DeserializeOwned>(_: &str) -> std::future::Ready<Result<T, ApiFailure>> {
    std::future::ready(Err(ApiFailure::Network))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn post_json<T: DeserializeOwned>(_: &str) -> std::future::Ready<Result<T, ApiFailure>> {
    std::future::ready(Err(ApiFailure::Network))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn post_json_body<B: Serialize, T: DeserializeOwned>(
    _: &str,
    _: &B,
) -> std::future::Ready<Result<T, ApiFailure>> {
    std::future::ready(Err(ApiFailure::Network))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn put_json_body<B: Serialize, T: DeserializeOwned>(
    _: &str,
    _: &B,
) -> std::future::Ready<Result<T, ApiFailure>> {
    std::future::ready(Err(ApiFailure::Network))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn delete_json<T: DeserializeOwned>(_: &str) -> std::future::Ready<Result<T, ApiFailure>> {
    std::future::ready(Err(ApiFailure::Network))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn navigate(_: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_encodes_the_return_path() {
        assert_eq!(login_url("/"), "/auth/login?return_to=%2F");
        assert_eq!(
            login_url("/docs/a b?x=1&y=é"),
            "/auth/login?return_to=%2Fdocs%2Fa%20b%3Fx%3D1%26y%3D%C3%A9"
        );
        assert_eq!(
            login_url("/a-b_c.d~e"),
            "/auth/login?return_to=%2Fa-b_c.d~e"
        );
    }
}
