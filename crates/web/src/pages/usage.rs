use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::auth::Me;
use shared::api::browse::VolumeInfo;
use shared::api::tasks::{Location, TaskState};
use shared::api::usage::UsageView;

use crate::api::{self, ApiFailure};
use crate::components::usage::{SECONDARY, ScanAgainButton};
use crate::components::{
    ErrorPanel, Shell, Sidebar, UsageHeader, UsageList, error_message, file_name,
};
use crate::format::{format_count, format_size};
use crate::routes::{Route, Segment, Segments};
use crate::tasks::{Task, use_tasks};

const TASK_NOT_FOUND: &str = "task_not_found";
const CARD: &str = "w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 shadow-sm dark:border-slate-800 dark:bg-slate-900";
const TITLE: &str = "text-lg font-semibold text-gray-900 dark:text-white";
const TEXT: &str = "mt-2 text-sm text-slate-600 dark:text-slate-400";

/// Where the space of a scanned folder goes: the scan's progress while it runs, then the
/// folder `path` of its result.
#[component]
pub fn Usage(id: String, volume: Segment, path: Segments) -> Element {
    let mut me = use_resource(|| api::get_json::<Me>("/api/me"));
    let mut volumes = use_resource(|| api::get_json::<Vec<VolumeInfo>>("/api/volumes"));
    let tasks = use_tasks();
    let list = tasks.list();
    let adopted = list.read().adopted();
    let task = list
        .read()
        .tasks()
        .iter()
        .find(|task| task.info.as_ref().is_some_and(|info| info.id == id))
        .cloned();
    // The result exists once the task is done; a task this tab does not know may be done too.
    let wanted = task
        .as_ref()
        .is_none_or(|task| task.state() == TaskState::Done);
    // Screen readers hear the scan start and end, not every count in between.
    let active = task.as_ref().is_some_and(Task::is_active);
    let mut watched = use_signal(|| false);
    use_effect(use_reactive!(|active| {
        if active {
            watched.set(true);
        }
    }));
    let url = api::usage_url(&id, &path.joined());
    let mut view = use_resource(use_reactive!(|url, wanted| async move {
        let result = if wanted {
            Some(api::get_json::<UsageView>(&url).await)
        } else {
            None
        };
        (url, result)
    }));

    let user = match me.read().clone() {
        Some(Ok(user)) => user,
        Some(Err(failure)) if !failure.is_unauthenticated() => {
            return rsx! {
                Shell { ErrorPanel { failure, on_retry: move |()| me.restart() } }
            };
        }
        _ => {
            return rsx! {
                Shell { p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} } }
            };
        }
    };
    let sidebar = match volumes.read().clone() {
        Some(Ok(volumes)) => rsx! {
            Sidebar { volumes, current: volume.as_str().to_owned() }
        },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            button {
                r#type: "button",
                class: "px-1 text-sm text-indigo-600 hover:underline dark:text-indigo-400",
                onclick: move |_| volumes.restart(),
                {t!("try-again")}
            }
        },
        _ => rsx! {},
    };

    // Scanning again covers what this page was about: the task's folder when known.
    let source = task.as_ref().map_or_else(
        || Location {
            volume: volume.as_str().to_owned(),
            path: path.joined(),
        },
        |task| task.spec.source().clone(),
    );
    let ready = *view.state().read() == UseResourceState::Ready;
    let fetched = view
        .read()
        .clone()
        .filter(|(fetched_url, _)| *fetched_url == url && ready)
        .and_then(|(_, result)| result);

    // The end of a watched scan is announced once, for the folder its result opened at.
    let mut finished_at = use_signal(|| None::<String>);
    let found = matches!(fetched, Some(Ok(_)));
    use_effect(use_reactive!(|found, url| {
        if found && watched() && finished_at.peek().is_none() {
            finished_at.set(Some(url));
        }
    }));
    let known_done = task
        .as_ref()
        .is_some_and(|task| task.state() == TaskState::Done);
    let gone = matches!(&fetched, Some(Err(failure)) if is_gone(failure));
    use_effect(use_reactive!(|gone, known_done, id| {
        if gone && known_done {
            tasks.result_gone(&id);
        }
    }));

    let status = if active {
        t!("usage-scanning", name: folder_name(&source))
    } else if finished_at.read().as_deref() == Some(url.as_str()) {
        t!("usage-scan-finished")
    } else {
        String::new()
    };
    let content = match (task, fetched) {
        (Some(task), _) if task.is_active() => rsx! {
            Scanning { task }
        },
        (Some(task), _) if task.result_dropped() => rsx! {
            Stopped { title: t!("usage-unavailable-title"), message: t!("usage-unavailable-message"), source }
        },
        (Some(task), _) if task.state() == TaskState::Cancelled => rsx! {
            Stopped { title: t!("usage-cancelled-title"), message: t!("usage-cancelled-message"), source }
        },
        (Some(task), _) if task.state() == TaskState::Failed => match task.failure() {
            Some(failure) if !is_gone(&failure) => rsx! {
                Failed { failure, source }
            },
            _ => rsx! {
                Stopped { title: t!("usage-unavailable-title"), message: t!("usage-unavailable-message"), source }
            },
        },
        (_, Some(Ok(found))) => rsx! {
            UsageHeader { id: id.clone(), view: found.clone() }
            UsageList { id: id.clone(), view: found }
        },
        // Before the tasks from before a reload are known, a running scan looks gone.
        (_, Some(Err(failure))) if is_gone(&failure) && adopted => rsx! {
            Stopped { title: t!("usage-unavailable-title"), message: t!("usage-unavailable-message"), source }
        },
        (_, Some(Err(failure))) if !failure.is_unauthenticated() && !is_gone(&failure) => rsx! {
            div { class: "flex justify-center pt-6",
                ErrorPanel { failure, on_retry: move |()| view.restart() }
            }
        },
        _ => rsx! {
            p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} }
        },
    };

    rsx! {
        Shell { user, sidebar,
            p { role: "status", class: "sr-only", {status} }
            div { class: "mx-auto w-full max-w-3xl", {content} }
        }
    }
}

fn folder_name(source: &Location) -> String {
    if source.path.is_empty() {
        source.volume.clone()
    } else {
        file_name(&source.path).to_owned()
    }
}

/// The server no longer has the task or its result.
fn is_gone(failure: &ApiFailure) -> bool {
    matches!(failure, ApiFailure::Server(body) if body.code == TASK_NOT_FOUND)
}

/// The scan's live counts while it runs.
#[component]
fn Scanning(task: Task) -> Element {
    let tasks = use_tasks();
    let locale = i18n().language();
    let source = task.spec.source().clone();
    let name = folder_name(&source);
    let (files, bytes) = task
        .info
        .as_ref()
        .map_or((0, 0), |info| (info.files_done, info.bytes_done));
    let key = task.key;

    rsx! {
        section { class: CARD,
            h1 { class: TITLE, {t!("usage-scanning", name: name)} }
            p { class: "{TEXT} tabular-nums",
                {t!("usage-files", count: files, files: format_count(files, &locale))}
                " · "
                {format_size(bytes, &locale)}
            }
            div { class: "mt-6 flex flex-wrap items-center gap-3",
                button {
                    r#type: "button",
                    class: SECONDARY,
                    onclick: move |_| tasks.cancel(key),
                    {t!("cancel")}
                }
            }
        }
    }
}

/// A scan that stopped or is gone, offering to scan `source` again.
#[component]
fn Stopped(title: String, message: String, source: Location) -> Element {
    let browse = Route::Browse {
        volume: source.volume.as_str().into(),
        path: Segments::of(&source.path),
    };
    rsx! {
        section { class: CARD,
            h1 { class: TITLE, {title} }
            p { class: TEXT, {message} }
            div { class: "mt-6 flex flex-wrap items-center gap-3",
                ScanAgainButton { source }
                Link { to: browse, class: SECONDARY, {t!("usage-open-in-explorer")} }
            }
        }
    }
}

/// A failed scan: why, the request id to report it with, and a new try.
#[component]
fn Failed(failure: ApiFailure, source: Location) -> Element {
    let request_id = match &failure {
        ApiFailure::Server(body) => body.request_id.clone(),
        _ => None,
    };
    rsx! {
        section { role: "alert", class: CARD,
            h1 { class: TITLE, {t!("usage-failed-title")} }
            p { class: TEXT, {error_message(&failure)} }
            if let Some(id) = request_id {
                p { class: "mt-2 font-mono text-xs text-slate-500 dark:text-slate-400",
                    {t!("error-reference", id: id)}
                }
            }
            div { class: "mt-6 flex flex-wrap items-center gap-3",
                ScanAgainButton { source }
            }
        }
    }
}
