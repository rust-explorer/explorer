use dioxus::prelude::*;
use dioxus_i18n::t;

use super::uploads_panel::{destination, file_name};
use crate::uploads::{Decision, ItemKey, use_uploads};

const TITLE_ID: &str = "conflict-title";
const APPLY_ALL_ID: &str = "conflict-apply-all";
const BUTTON: &str = "rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";
const PRIMARY: &str = "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600";

/// Asks what to do with the first upload whose destination already exists.
#[component]
pub fn ConflictDialog() -> Element {
    let queue = use_uploads().queue();
    let queue = queue.read();
    let Some(item) = queue.pending_decision() else {
        return rsx! {};
    };
    let (volume, path) = destination(item);
    let folder = match path.rsplit_once('/') {
        Some((parent, _)) => format!("{volume}/{parent}"),
        None => volume.to_owned(),
    };

    rsx! {
        // Keyed by item so the next conflict starts unchecked and takes focus again.
        ConflictPrompt {
            key: "{item.key.0}",
            item_key: item.key,
            name: file_name(path).to_owned(),
            folder,
            several: queue.batch_len(item.batch) > 1,
        }
    }
}

#[component]
fn ConflictPrompt(item_key: ItemKey, name: String, folder: String, several: bool) -> Element {
    let uploads = use_uploads();
    let mut apply_to_all = use_signal(|| false);
    let decide = move |decision| uploads.decide(item_key, decision, apply_to_all());

    rsx! {
        div { class: "fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4",
            div {
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: TITLE_ID,
                class: "w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 shadow-xl dark:border-slate-800 dark:bg-slate-900",
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        decide(Decision::Cancel);
                    }
                },
                h2 {
                    id: TITLE_ID,
                    class: "break-words text-sm font-medium text-gray-900 dark:text-white",
                    {t!("conflict-title", name: name, folder: folder)}
                }
                if several {
                    div { class: "mt-4 flex items-center gap-2",
                        input {
                            id: APPLY_ALL_ID,
                            r#type: "checkbox",
                            class: "size-4 rounded border-slate-300 text-indigo-600",
                            checked: apply_to_all(),
                            onchange: move |event| apply_to_all.set(event.checked()),
                        }
                        label {
                            r#for: APPLY_ALL_ID,
                            class: "text-sm text-slate-700 dark:text-slate-300",
                            {t!("conflict-apply-all")}
                        }
                    }
                }
                div { class: "mt-5 flex flex-wrap justify-end gap-3",
                    button {
                        r#type: "button",
                        class: BUTTON,
                        onclick: move |_| decide(Decision::Cancel),
                        {t!("conflict-cancel")}
                    }
                    button {
                        r#type: "button",
                        class: BUTTON,
                        onclick: move |_| decide(Decision::Replace),
                        {t!("conflict-replace")}
                    }
                    button {
                        r#type: "button",
                        class: PRIMARY,
                        onmounted: move |event| async move {
                            let _ = event.data().set_focus(true).await;
                        },
                        onclick: move |_| decide(Decision::KeepBoth),
                        {t!("conflict-keep-both")}
                    }
                }
            }
        }
    }
}
