use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::CreateFolder;

use crate::api::{self, ApiFailure};
use crate::components::common::icons::FolderPlusIcon;
use crate::components::common::{FocusTarget, Modal, error_message};
use crate::names::{NameError, validate_name};
use crate::revision::Revision;

const TITLE_ID: &str = "new-folder-title";
const INPUT_ID: &str = "new-folder-name";
const ERROR_ID: &str = "new-folder-error";
pub(super) const BUTTON: &str = "rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";
pub(super) const INPUT: &str = "mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-900 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:bg-slate-950 dark:text-white";
pub(super) const PRIMARY: &str = "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:bg-slate-300 dark:disabled:bg-slate-700";

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

pub(super) enum Problem {
    Name(NameError),
    Server(ApiFailure),
}

impl Problem {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Name(NameError::Empty) => t!("name-error-empty"),
            Self::Name(NameError::Slash) => t!("name-error-slash"),
            Self::Name(NameError::Dot) => t!("name-error-dot"),
            Self::Name(NameError::Reserved) => t!("name-error-reserved"),
            Self::Name(NameError::TooLong) => t!("name-error-too-long"),
            Self::Server(failure) => error_message(failure),
        }
    }
}

/// Asks for a name and creates that folder inside `path`; the dialog stays open on an error.
#[component]
pub fn NewFolderDialog(
    volume: String,
    path: String,
    on_close: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
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
        spawn(async move {
            match api::post_json_body::<_, ()>(&url, &body).await {
                Ok(()) => {
                    *revision.write() += 1;
                    on_close.call(());
                }
                Err(failure) => {
                    busy.set(false);
                    problem.set(Some(Problem::Server(failure)));
                }
            }
        });
    };

    let message = problem.read().as_ref().map(Problem::message);

    rsx! {
        Modal { labelled_by: TITLE_ID, on_close: move |()| on_close.call(()), return_focus,
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
                    oninput: move |event| name.set(event.value()),
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
                        onclick: move |_| on_close.call(()),
                        {t!("cancel")}
                    }
                    button { r#type: "submit", class: PRIMARY, disabled: busy(), {t!("new-folder-create")} }
                }
            }
        }
    }
}
