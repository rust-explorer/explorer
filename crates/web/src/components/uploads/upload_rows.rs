use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;

use crate::components::common::icons::{CheckIcon, CloseIcon};
use crate::components::common::{FailureNote, ICON_BUTTON, file_name};
use crate::format::{format_size, format_speed, now_ms};
use crate::uploads::{Item, Kind, Status, use_uploads};

/// Volume and path the item is uploaded to.
pub fn destination(item: &Item) -> (&str, &str) {
    match &item.kind {
        Kind::File(spec) => (&spec.volume, &spec.path),
        Kind::Folder { volume, path } => (volume, path),
    }
}

fn is_finished(status: &Status) -> bool {
    matches!(
        status,
        Status::Done | Status::Failed(_) | Status::Cancelled | Status::Skipped
    )
}

/// Uploads that are not finished yet.
pub fn active_uploads(items: &[Item]) -> usize {
    items
        .iter()
        .filter(|item| !is_finished(&item.status))
        .count()
}

/// The upload rows of the activity panel.
#[component]
pub fn UploadRows() -> Element {
    let queue = use_uploads().queue();
    let items = queue.read().items().to_vec();

    rsx! {
        for item in items {
            UploadRow { key: "{item.key.0}", item }
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
        (
            Status::Retrying {
                at_ms,
                connection_lost,
                ..
            },
            _,
        ) => rsx! {
            RetryCountdown { at_ms: *at_ms, connection_lost: *connection_lost }
        },
        (Status::Failed(failure), _) => rsx! {
            FailureNote { failure: failure.clone(), upload: true }
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
fn RetryCountdown(at_ms: i64, connection_lost: bool) -> Element {
    let mut now = use_signal(now_ms);
    use_future(move || async move {
        loop {
            sleep_ms(1000).await;
            now.set(now_ms());
        }
    });
    let seconds = ((at_ms - now()) + 999).div_euclid(1000).max(1);
    let message = if connection_lost {
        t!("uploads-retrying", seconds: seconds)
    } else {
        t!("uploads-retrying-server", seconds: seconds)
    };

    rsx! {
        p { class: "mt-0.5 text-xs text-amber-700 dark:text-amber-400", {message} }
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
