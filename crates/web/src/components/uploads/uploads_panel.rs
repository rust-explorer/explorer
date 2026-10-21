use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;

use crate::api::ApiFailure;
use crate::components::common::error_message;
use crate::components::common::icons::{CheckIcon, CloseIcon, MinusIcon, PlusIcon};
use crate::format::{format_size, format_speed, now_ms};
use crate::uploads::{Item, Kind, Status, use_uploads};

const LIST_ID: &str = "uploads-list";
const ICON_BUTTON: &str = "inline-flex size-6 items-center justify-center rounded text-slate-400 hover:bg-slate-100 hover:text-slate-600 focus-visible:outline-2 focus-visible:outline-indigo-600 dark:hover:bg-slate-800 dark:hover:text-slate-300";

/// Volume and path the item is uploaded to.
pub fn destination(item: &Item) -> (&str, &str) {
    match &item.kind {
        Kind::File(spec) => (&spec.volume, &spec.path),
        Kind::Folder { volume, path } => (volume, path),
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_finished(status: &Status) -> bool {
    matches!(
        status,
        Status::Done | Status::Failed(_) | Status::Cancelled | Status::Skipped
    )
}

#[component]
pub fn UploadsPanel() -> Element {
    let uploads = use_uploads();
    let queue = uploads.queue();
    let mut collapsed = use_signal(|| false);
    let queue = queue.read();
    let skipped = queue.skipped();
    if queue.items().is_empty() && skipped == 0 {
        return rsx! {};
    }
    let active = queue
        .items()
        .iter()
        .filter(|item| !is_finished(&item.status))
        .count();
    let collapse_label = if collapsed() {
        t!("uploads-expand")
    } else {
        t!("uploads-collapse")
    };

    rsx! {
        section {
            class: "fixed inset-x-4 bottom-20 z-30 rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm shadow-lg md:inset-x-auto md:bottom-4 md:right-4 md:w-80 dark:border-slate-700 dark:bg-slate-900",
            aria_label: t!("uploads-title", count: active),
            div { class: "flex items-center gap-1",
                h2 { class: "flex-1 font-semibold text-gray-900 dark:text-white",
                    {t!("uploads-title", count: active)}
                }
                button {
                    r#type: "button",
                    class: ICON_BUTTON,
                    aria_label: "{collapse_label}",
                    title: "{collapse_label}",
                    aria_expanded: !collapsed(),
                    aria_controls: LIST_ID,
                    onclick: move |_| collapsed.toggle(),
                    if collapsed() {
                        PlusIcon { class: "size-4" }
                    } else {
                        MinusIcon {}
                    }
                }
                if active == 0 {
                    button {
                        r#type: "button",
                        class: ICON_BUTTON,
                        aria_label: t!("uploads-close"),
                        title: t!("uploads-close"),
                        onclick: move |_| uploads.clear_finished(),
                        CloseIcon {}
                    }
                }
            }
            if skipped > 0 {
                p { class: "mt-1 text-xs text-amber-700 dark:text-amber-400",
                    {t!("uploads-unreadable", count: skipped)}
                }
            }
            if !collapsed() {
                ul { id: LIST_ID, class: "mt-1 max-h-80 overflow-y-auto",
                    for item in queue.items().iter().cloned() {
                        UploadRow { key: "{item.key.0}", item }
                    }
                }
            }
        }
    }
}

#[component]
fn UploadRow(item: Item) -> Element {
    let uploads = use_uploads();
    let (volume, path) = destination(&item);
    let full_path = format!("{volume}/{path}");
    let name = file_name(path).to_owned();
    let key = item.key;

    rsx! {
        li { class: "border-t border-slate-100 py-2 first:border-t-0 dark:border-slate-800",
            div { class: "flex items-center gap-2",
                span {
                    class: "min-w-0 flex-1 truncate text-gray-900 dark:text-slate-100",
                    title: "{full_path}",
                    {name.clone()}
                }
                if item.status == Status::Done {
                    span {
                        class: "text-green-600 dark:text-green-400",
                        role: "img",
                        aria_label: t!("uploads-done"),
                        CheckIcon {}
                    }
                }
                if !is_finished(&item.status) {
                    button {
                        r#type: "button",
                        class: ICON_BUTTON,
                        aria_label: t!("uploads-cancel", name: name.clone()),
                        title: t!("uploads-cancel", name: name),
                        onclick: move |_| uploads.cancel(key),
                        CloseIcon { class: "size-3.5" }
                    }
                }
            }
            UploadStatus { item }
        }
    }
}

#[component]
fn UploadStatus(item: Item) -> Element {
    let uploads = use_uploads();
    let locale = i18n().language();
    let note = "mt-0.5 text-xs text-slate-500 dark:text-slate-400";

    match (&item.status, &item.kind) {
        (Status::Uploading, Kind::File(spec)) => {
            let percent = (u128::from(item.sent) * 100)
                .checked_div(u128::from(spec.size))
                .map_or(100, |percent| percent.min(100));
            let mut parts = vec![t!(
                "uploads-sent",
                sent: format_size(item.sent, &locale),
                total: format_size(spec.size, &locale)
            )];
            if let Some(speed) = uploads.speed(item.key) {
                parts.push(format_speed(speed, &locale));
            }
            parts.push(t!("uploads-resumable"));
            rsx! {
                div {
                    class: "mt-1 h-1 rounded-full bg-slate-200 dark:bg-slate-700",
                    role: "progressbar",
                    aria_label: t!("uploads-progress"),
                    aria_valuemin: "0",
                    aria_valuemax: "100",
                    aria_valuenow: "{percent}",
                    div {
                        class: "h-1 rounded-full bg-indigo-600 dark:bg-indigo-400",
                        width: "{percent}%",
                    }
                }
                p { class: note, {parts.join(" · ")} }
            }
        }
        (Status::Uploading, Kind::Folder { .. }) => rsx! {
            p { class: note, {t!("uploads-creating-folder")} }
        },
        (Status::Queued, _) => rsx! {
            p { class: note, {t!("uploads-waiting")} }
        },
        (Status::NeedsDecision, _) => rsx! {
            p { class: note, {t!("uploads-needs-decision")} }
        },
        (Status::Retrying { at_ms, .. }, _) => rsx! {
            RetryCountdown { at_ms: *at_ms }
        },
        (Status::Failed(failure), _) => rsx! {
            FailureNote { failure: failure.clone() }
        },
        (Status::Cancelled, _) => rsx! {
            p { class: note, {t!("uploads-cancelled")} }
        },
        (Status::Skipped, _) => rsx! {
            p { class: note, {t!("uploads-skipped")} }
        },
        (Status::Done, _) => rsx! {},
    }
}

#[component]
fn RetryCountdown(at_ms: i64) -> Element {
    let mut now = use_signal(now_ms);
    use_future(move || async move {
        loop {
            sleep_ms(1000).await;
            now.set(now_ms());
        }
    });
    let seconds = ((at_ms - now()) + 999).div_euclid(1000).max(1);

    rsx! {
        p { class: "mt-0.5 text-xs text-amber-700 dark:text-amber-400",
            {t!("uploads-retrying", seconds: seconds)}
        }
    }
}

#[component]
fn FailureNote(failure: ApiFailure) -> Element {
    let reference = match &failure {
        ApiFailure::Server(body) => body.request_id.clone(),
        _ => None,
    };

    rsx! {
        p { class: "mt-0.5 text-xs text-red-700 dark:text-red-400", {error_message(&failure)} }
        if let Some(id) = reference {
            p { class: "font-mono text-xs text-slate-500 dark:text-slate-400",
                {t!("error-reference", id: id)}
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn sleep_ms(ms: u32) {
    gloo_timers::future::TimeoutFuture::new(ms).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn sleep_ms(_: u32) {
    std::future::pending::<()>().await;
}
