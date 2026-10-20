use std::fmt::Write as _;

use serde::de::DeserializeOwned;
use shared::api::{ErrorBody, LogoutResponse};

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

/// Same character set as `encodeURIComponent`.
pub fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

pub fn entries_url(volume: &str, path: &str) -> String {
    format!(
        "/api/volumes/{}/entries?path={}",
        encode_component(volume),
        encode_component(path)
    )
}

pub fn download_url(volume: &str, path: &str) -> String {
    files_url("download", volume, path)
}

pub fn zip_url(volume: &str, path: &str) -> String {
    files_url("zip", volume, path)
}

#[allow(dead_code)]
pub fn upload_url(id: &str) -> String {
    format!("/files/uploads/{}", encode_component(id))
}

#[allow(dead_code)]
pub fn folders_url(volume: &str) -> String {
    format!("/api/volumes/{}/folders", encode_component(volume))
}

fn files_url(action: &str, volume: &str, path: &str) -> String {
    format!(
        "/files/{action}?volume={}&path={}",
        encode_component(volume),
        encode_component(path)
    )
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
    if response.status() == 401 {
        navigate(&login_url(&current_path_and_query()));
        return Err(ApiFailure::Server(ErrorBody {
            code: UNAUTHENTICATED.to_owned(),
            message: String::new(),
            details: None,
            request_id: None,
        }));
    }
    let status = response.status();
    match response.json().await {
        Ok(body) => Err(ApiFailure::Server(body)),
        Err(_) => Err(ApiFailure::Http(status)),
    }
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

    #[test]
    fn urls_percent_encode() {
        assert_eq!(
            entries_url("media", "é #?%/x"),
            "/api/volumes/media/entries?path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            download_url("media", "é #?%/x"),
            "/files/download?volume=media&path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            zip_url("media", "é #?%/x"),
            "/files/zip?volume=media&path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            entries_url("my files&?", ""),
            "/api/volumes/my%20files%26%3F/entries?path="
        );
        assert_eq!(
            download_url("a&b", "c&d=1"),
            "/files/download?volume=a%26b&path=c%26d%3D1"
        );
    }
}
