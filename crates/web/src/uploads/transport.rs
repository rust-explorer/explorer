use std::cell::Cell;
use std::rc::Rc;

use gloo_net::http::{Method, RequestBuilder, Response};
use gloo_timers::callback::Interval;
use shared::api::browse::CreateFolder;
use shared::api::conflict::ConflictPolicy;
use shared::api::error::ErrorBody;
use shared::tus::{TUS_VERSION, encode_metadata};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{File, ProgressEvent, RequestCredentials, XmlHttpRequest};

use super::FileSpec;
use crate::api::{self, ApiFailure};

const UPLOADS_URL: &str = "/files/uploads";
const UPLOAD_OFFSET: &str = "Upload-Offset";
const UPLOAD_NOT_FOUND: &str = "upload_not_found";
/// A PATCH that reports no progress for this long is a half-open connection.
const STALL_TIMEOUT_MS: f64 = 120_000.0;
const STALL_CHECK_MS: u32 = 5_000;

fn tus(method: Method, url: &str) -> RequestBuilder {
    RequestBuilder::new(url)
        .method(method)
        .credentials(RequestCredentials::SameOrigin)
        .header("Tus-Resumable", TUS_VERSION)
        .header("Accept", "application/json")
}

async fn checked(request: Result<Response, gloo_net::Error>) -> Result<Response, ApiFailure> {
    let response = request.map_err(|_| ApiFailure::Network)?;
    if response.ok() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(api::failure(response.status(), &body))
}

fn offset(response: &Response) -> Result<u64, ApiFailure> {
    response
        .headers()
        .get(UPLOAD_OFFSET)
        .and_then(|value| value.parse().ok())
        .ok_or(ApiFailure::Network)
}

/// Starts an upload and returns its URL.
pub async fn create(spec: &FileSpec, conflict: ConflictPolicy) -> Result<String, ApiFailure> {
    let modified = spec.modified_ms.map(|ms| ms.to_string());
    let mut pairs = vec![
        ("volume", spec.volume.as_str()),
        ("path", spec.path.as_str()),
    ];
    if let Some(modified) = &modified {
        pairs.push(("modified", modified));
    }
    pairs.push(("conflict", conflict.as_str()));
    let request = tus(Method::POST, UPLOADS_URL)
        .header("Upload-Length", &spec.size.to_string())
        .header("Upload-Metadata", &encode_metadata(&pairs))
        .send()
        .await;
    checked(request)
        .await?
        .headers()
        .get("Location")
        .ok_or(ApiFailure::Network)
}

/// The offset the server has for the upload.
pub async fn head(url: &str) -> Result<u64, ApiFailure> {
    let response = tus(Method::HEAD, url)
        .send()
        .await
        .map_err(|_| ApiFailure::Network)?;
    match response.status() {
        // A HEAD response has no body to carry the error code.
        404 => Err(ApiFailure::Server(ErrorBody {
            code: UPLOAD_NOT_FOUND.to_owned(),
            message: String::new(),
            details: None,
            request_id: None,
        })),
        _ if response.ok() => offset(&response),
        status => Err(api::failure(status, "")),
    }
}

pub async fn delete(url: &str) -> Result<(), ApiFailure> {
    checked(tus(Method::DELETE, url).send().await)
        .await
        .map(drop)
}

pub async fn create_folder(volume: &str, path: &str) -> Result<(), ApiFailure> {
    let request = RequestBuilder::new(&api::folders_url(volume))
        .method(Method::POST)
        .credentials(RequestCredentials::SameOrigin)
        .header("Accept", "application/json")
        .json(&CreateFolder {
            path: path.to_owned(),
            exclusive: false,
        })
        .map_err(|_| ApiFailure::Network)?
        .send()
        .await;
    checked(request).await.map(drop)
}

/// A PATCH of one chunk, sent with XHR for its upload progress. It is aborted, and ends as a
/// network failure, when no progress is reported for [`STALL_TIMEOUT_MS`].
pub struct Patch {
    xhr: XmlHttpRequest,
    done: js_sys::Promise,
    _progress: Closure<dyn FnMut(ProgressEvent)>,
    _watchdog: Interval,
}

impl Patch {
    /// Sends `len` bytes of `file` from `offset`, possibly none; `on_progress` gets the bytes of
    /// this chunk sent.
    pub fn send(
        file: &File,
        url: &str,
        offset: u64,
        len: u64,
        mut on_progress: impl FnMut(u64) + 'static,
    ) -> Result<Self, ApiFailure> {
        let xhr = XmlHttpRequest::new().map_err(|_| ApiFailure::Network)?;
        xhr.open_with_async("PATCH", url, true)
            .map_err(|_| ApiFailure::Network)?;
        for (name, value) in [
            ("Tus-Resumable", TUS_VERSION.to_owned()),
            ("Content-Type", "application/offset+octet-stream".to_owned()),
            ("Accept", "application/json".to_owned()),
            (UPLOAD_OFFSET, offset.to_string()),
        ] {
            xhr.set_request_header(name, &value)
                .map_err(|_| ApiFailure::Network)?;
        }
        let last_progress = Rc::new(Cell::new(js_sys::Date::now()));
        let progress = Closure::<dyn FnMut(ProgressEvent)>::new({
            let last_progress = Rc::clone(&last_progress);
            move |event: ProgressEvent| {
                last_progress.set(js_sys::Date::now());
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "loaded is a whole, non-negative byte count"
                )]
                on_progress(event.loaded() as u64);
            }
        });
        xhr.upload()
            .map_err(|_| ApiFailure::Network)?
            .set_onprogress(Some(progress.as_ref().unchecked_ref()));
        let done = js_sys::Promise::new(&mut |resolve, _| xhr.set_onloadend(Some(&resolve)));
        #[expect(
            clippy::cast_precision_loss,
            reason = "file offsets stay far below 2^53"
        )]
        let chunk = file
            .slice_with_f64_and_f64(offset as f64, (offset + len) as f64)
            .map_err(|_| ApiFailure::Network)?;
        xhr.send_with_opt_blob(Some(&chunk))
            .map_err(|_| ApiFailure::Network)?;
        let watchdog = Interval::new(STALL_CHECK_MS, {
            let xhr = xhr.clone();
            move || {
                if js_sys::Date::now() - last_progress.get() >= STALL_TIMEOUT_MS {
                    let _ = xhr.abort();
                }
            }
        });
        Ok(Self {
            xhr,
            done,
            _progress: progress,
            _watchdog: watchdog,
        })
    }

    pub fn request(&self) -> XmlHttpRequest {
        self.xhr.clone()
    }

    /// The server's new offset once the request ends.
    pub async fn finish(self) -> Result<u64, ApiFailure> {
        let _ = wasm_bindgen_futures::JsFuture::from(self.done).await;
        let xhr = self.xhr;
        match xhr.status().unwrap_or(0) {
            0 => Err(ApiFailure::Network),
            200..=299 => xhr
                .get_response_header(UPLOAD_OFFSET)
                .ok()
                .flatten()
                .and_then(|value| value.parse().ok())
                .ok_or(ApiFailure::Network),
            status => Err(api::failure(
                status,
                &xhr.response_text().ok().flatten().unwrap_or_default(),
            )),
        }
    }
}
