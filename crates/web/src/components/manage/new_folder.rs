use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::CreateFolder;

use super::form::{BUTTON, INPUT, PRIMARY, Problem};
use crate::api;
use crate::components::common::icons::FolderPlusIcon;
use crate::components::common::{FocusTarget, Modal};
use crate::names::validate_name;
use crate::revision::Revision;

const TITLE_ID: &str = "new-folder-title";
const INPUT_ID: &str = "new-folder-name";
const ERROR_ID: &str = "new-folder-error";
/// The header's New folder button and its dialog; below `md` the floating "+" menu opens it.
#[component]
pub fn NewFolderButton(volume: String, path: String, read_only: bool) -> Element {
    let mut open = use_signal(|| false);
    if read_only {
        return rsx! {};
    }

    rsx! {
        button {
            r#type: "button",
            class: "hidden items-center gap-1.5 rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 md:inline-flex dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800",
            onclick: move |_| open.set(true),
            FolderPlusIcon {}
            {t!("new-folder")}
        }
        if open() {
            NewFolderDialog { volume, path, on_close: move |()| open.set(false) }
        }
    }
}

/// Asks for a name and creates that folder inside `path`; the dialog stays open on an error.
/// The folder is the one shown when the dialog opened, even if the browser navigates meanwhile.
#[component]
pub fn NewFolderDialog(
    volume: String,
    path: String,
    on_close: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
    let (volume, path) = use_hook(move || (volume, path));
    let Revision(mut revision) = use_context();
    let mut name = use_signal(String::new);
    let mut problem = use_signal(|| None::<Problem>);
    let mut busy = use_signal(|| false);

    let submit = move |event: FormEvent| {
        event.prevent_default();
        if busy() {
            return;
        }
        let name = name();
        let at_root = path.is_empty();
        let name = match validate_name(&name, at_root) {
            Ok(name) => name.to_owned(),
            Err(error) => {
                problem.set(Some(Problem::Name(error)));
                return;
            }
        };
        let body = CreateFolder {
            path: if at_root {
                name
            } else {
                format!("{path}/{name}")
            },
            exclusive: true,
        };
        let url = api::folders_url(&volume);
        busy.set(true);
        problem.set(None);
        // Runs to its end even if the dialog closes meanwhile, then refreshes the listing.
        spawn_forever(async move {
            let result = api::post_json_body::<_, ()>(&url, &body).await;
            let closed = busy.try_peek().is_err();
            if result.is_ok() || closed {
                *revision.write() += 1;
            }
            if closed {
                return;
            }
            match result {
                Ok(()) => on_close.call(()),
                Err(failure) => {
                    busy.set(false);
                    problem.set(Some(Problem::Server(failure)));
                }
            }
        });
    };

    let message = problem.read().as_ref().map(Problem::message);

    // A request can hang (a stalled network or mount): closing stays possible, and the listing
    // is refreshed once the request ends.
    let close = move |()| on_close.call(());

    rsx! {
        Modal { labelled_by: TITLE_ID, on_close: close, return_focus,
            form { onsubmit: submit,
                h2 {
                    id: TITLE_ID,
                    class: "text-sm font-medium text-gray-900 dark:text-white",
                    {t!("new-folder-title")}
                }
                label {
                    r#for: INPUT_ID,
                    class: "mt-4 block text-sm text-slate-700 dark:text-slate-300",
                    {t!("new-folder-name")}
                }
                input {
                    id: INPUT_ID,
                    r#type: "text",
                    class: INPUT,
                    autofocus: true,
                    autocomplete: "off",
                    spellcheck: false,
                    value: name(),
                    aria_invalid: message.is_some(),
                    aria_describedby: if message.is_some() { ERROR_ID },
                    oninput: move |event| {
                        name.set(event.value());
                        problem.set(None);
                    },
                }
                if let Some(message) = message.clone() {
                    p {
                        id: ERROR_ID,
                        role: "alert",
                        class: "mt-2 text-sm text-red-600 dark:text-red-400",
                        {message}
                    }
                }
                div { class: "mt-5 flex justify-end gap-3",
                    button {
                        r#type: "button",
                        class: BUTTON,
                        onclick: move |_| close(()),
                        {t!("cancel")}
                    }
                    button { r#type: "submit", class: PRIMARY, disabled: busy(), {t!("new-folder-create")} }
                }
            }
        }
    }
}
