use dioxus::prelude::*;
use dioxus_i18n::{t, te};
use shared::api::error::ErrorBody;

use crate::api::ApiFailure;
use crate::favorites::MAX_FAVORITES;
use crate::routes::{Route, Segments};

const BUTTON_CLASS: &str =
    "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500";
const LINK_CLASS: &str = "rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";

fn detail<'a>(body: &'a ErrorBody, field: &str) -> Option<&'a str> {
    body.details.as_ref()?.get(field)?.as_str()
}

/// `volume/path` from the error details, as the server writes it in its messages.
fn detail_path(body: &ErrorBody) -> String {
    match (detail(body, "volume"), detail(body, "path")) {
        (Some(volume), Some(path)) if !path.is_empty() => format!("{volume}/{path}"),
        (Some(volume), _) => volume.to_owned(),
        (None, path) => path.unwrap_or_default().to_owned(),
    }
}

/// The translated message for `failure`.
pub fn error_message(failure: &ApiFailure) -> String {
    match failure {
        ApiFailure::Network => t!("load-failed"),
        ApiFailure::Http(_) => t!("error-unknown"),
        ApiFailure::Server(body) if body.code == "favorites_full" => {
            t!("error-favorites_full", max: MAX_FAVORITES)
        }
        ApiFailure::Server(body) => te!(&format!("error-{}", body.code), path: detail_path(body))
            .ok()
            .or_else(|| (!body.message.is_empty()).then(|| body.message.clone()))
            .unwrap_or_else(|| t!("error-unknown")),
    }
}

/// Like [`error_message`], worded for an upload.
pub fn upload_error_message(failure: &ApiFailure) -> String {
    match failure {
        ApiFailure::Http(413) => t!("uploads-error-too-large"),
        ApiFailure::Server(body) if body.code == "permission_denied" => {
            t!("uploads-error-permission_denied", path: detail_path(body))
        }
        _ => error_message(failure),
    }
}

#[component]
pub fn ErrorPanel(failure: ApiFailure, on_retry: EventHandler<()>) -> Element {
    let body = match &failure {
        ApiFailure::Server(body) => Some(body),
        _ => None,
    };
    let request_id = body.and_then(|body| body.request_id.clone());
    let volume_root = body
        .filter(|body| matches!(body.code.as_str(), "path_not_found" | "not_a_directory"))
        .and_then(|body| detail(body, "volume"))
        .map(ToOwned::to_owned);
    let unknown_volume = body.is_some_and(|body| body.code == "volume_not_found");

    rsx! {
        section {
            role: "alert",
            class: "w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 shadow-sm dark:border-slate-800 dark:bg-slate-900",
            p { class: "text-sm font-medium text-gray-900 dark:text-white", {error_message(&failure)} }
            if let Some(id) = request_id {
                p { class: "mt-2 font-mono text-xs text-slate-500 dark:text-slate-400",
                    {t!("error-reference", id: id)}
                }
            }
            div { class: "mt-5 flex flex-wrap items-center gap-3",
                button {
                    r#type: "button",
                    class: BUTTON_CLASS,
                    onclick: move |_| on_retry.call(()),
                    {t!("try-again")}
                }
                if let Some(volume) = volume_root {
                    Link {
                        to: Route::Browse { volume: volume.clone().into(), path: Segments::default() },
                        class: LINK_CLASS,
                        {t!("error-open-volume", volume: volume)}
                    }
                }
                if unknown_volume {
                    Link { to: Route::Home {}, class: LINK_CLASS, {t!("error-go-home")} }
                }
            }
        }
    }
}

/// A failure in a list row: its message and the request id to report it with.
#[component]
pub fn FailureNote(failure: ApiFailure, #[props(default)] upload: bool) -> Element {
    let reference = match &failure {
        ApiFailure::Server(body) => body.request_id.clone(),
        _ => None,
    };
    let message = if upload {
        upload_error_message(&failure)
    } else {
        error_message(&failure)
    };

    rsx! {
        p { class: "mt-0.5 text-xs text-red-700 dark:text-red-400", {message} }
        if let Some(id) = reference {
            p { class: "font-mono text-xs text-slate-500 dark:text-slate-400",
                {t!("error-reference", id: id)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn body(details: Option<serde_json::Value>) -> ErrorBody {
        ErrorBody {
            code: "path_not_found".to_owned(),
            message: "Not found: media/Films".to_owned(),
            details,
            request_id: None,
        }
    }

    #[test]
    fn detail_path_joins_volume_and_path() {
        assert_eq!(
            detail_path(&body(Some(json!({"volume": "media", "path": "Films/x"})))),
            "media/Films/x"
        );
        assert_eq!(
            detail_path(&body(Some(json!({"volume": "media", "path": ""})))),
            "media"
        );
        assert_eq!(
            detail_path(&body(Some(json!({"volume": "media"})))),
            "media"
        );
        assert_eq!(detail_path(&body(Some(json!({"volume": 3})))), "");
        assert_eq!(detail_path(&body(None)), "");
    }
}
