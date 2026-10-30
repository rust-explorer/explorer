use gloo_net::http::Request;
use gloo_timers::callback::Timeout;
use shared::api::tasks::{TaskInfo, TaskSpec};
use web_sys::RequestCredentials;

use crate::api::{self, ApiFailure};

pub async fn create(spec: &TaskSpec) -> Result<TaskInfo, ApiFailure> {
    api::post_json_body(api::TASKS_URL, spec).await
}

/// A poll that gets no answer stops polling, so it fails after this long and the next one goes.
const LIST_TIMEOUT_MS: u32 = 10_000;

pub async fn list() -> Result<Vec<TaskInfo>, ApiFailure> {
    let guard = api::AbortGuard::new();
    let signal = guard.signal();
    let _abort = Timeout::new(LIST_TIMEOUT_MS, move || drop(guard));
    api::get_json_abortable(api::TASKS_URL, &signal).await
}

/// Cancels the task, or dismisses it once finished.
pub async fn delete(id: &str) -> Result<(), ApiFailure> {
    let response = Request::delete(&api::task_url(id))
        .credentials(RequestCredentials::SameOrigin)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| ApiFailure::Network)?;
    if response.ok() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(api::failure(response.status(), &body))
}
