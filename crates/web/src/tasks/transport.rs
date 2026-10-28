use gloo_net::http::Request;
use shared::api::tasks::{TaskInfo, TaskSpec};
use web_sys::RequestCredentials;

use crate::api::{self, ApiFailure};

pub async fn create(spec: &TaskSpec) -> Result<TaskInfo, ApiFailure> {
    api::post_json_body(api::TASKS_URL, spec).await
}

pub async fn list() -> Result<Vec<TaskInfo>, ApiFailure> {
    api::get_json(api::TASKS_URL).await
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
