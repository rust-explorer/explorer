use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;

use super::icons::{ArrowUpIcon, PlusIcon};
use crate::uploads::{Uploads, use_uploads};

const READ_ONLY_ID: &str = "upload-read-only";
const MENU_ITEM: &str = "block w-full rounded-md px-3 py-1.5 text-left text-sm text-slate-700 hover:bg-slate-100 focus-visible:bg-slate-100 focus-visible:outline-none dark:text-slate-200 dark:hover:bg-slate-800 dark:focus-visible:bg-slate-800";

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Files,
    Folder,
}

/// The header's Upload button, a floating "+" below `md`, and the file pickers behind them.
#[component]
pub fn UploadButton(volume: String, path: String, read_only: bool) -> Element {
    let uploads = use_uploads();
    let mut files_input = use_signal(|| None::<Rc<MountedData>>);
    let mut folder_input = use_signal(|| None::<Rc<MountedData>>);

    let open_picker = move |picker| {
        let input = match picker {
            Picker::Files => files_input(),
            Picker::Folder => folder_input(),
        };
        if let Some(input) = input {
            click(&input);
        }
    };
    let on_files = {
        let (volume, path) = (volume.clone(), path.clone());
        move |event: FormEvent| picked(uploads, &volume, &path, &event, files_input().as_deref())
    };
    let on_folder =
        move |event: FormEvent| picked(uploads, &volume, &path, &event, folder_input().as_deref());

    rsx! {
        input {
            r#type: "file",
            class: "hidden",
            multiple: true,
            tabindex: "-1",
            aria_hidden: "true",
            onmounted: move |event| files_input.set(Some(event.data())),
            onchange: on_files,
        }
        input {
            r#type: "file",
            class: "hidden",
            "webkitdirectory": "true",
            tabindex: "-1",
            aria_hidden: "true",
            onmounted: move |event| folder_input.set(Some(event.data())),
            onchange: on_folder,
        }
        if read_only {
            span { id: READ_ONLY_ID, class: "sr-only", {t!("upload-read-only")} }
        }
        div { class: "hidden md:block",
            UploadMenu { floating: false, read_only, on_pick: open_picker }
        }
        div { class: "fixed bottom-4 right-4 z-30 md:hidden",
            UploadMenu { floating: true, read_only, on_pick: open_picker }
        }
    }
}

#[component]
fn UploadMenu(floating: bool, read_only: bool, on_pick: EventHandler<Picker>) -> Element {
    let mut open = use_signal(|| false);
    let mut trigger = use_signal(|| None::<Rc<MountedData>>);
    let menu_id = if floating {
        "upload-menu-floating"
    } else {
        "upload-menu"
    };
    let (button_class, menu_class) = if floating {
        (
            "inline-flex size-14 items-center justify-center rounded-full bg-indigo-600 text-white shadow-lg hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:bg-slate-400 dark:disabled:bg-slate-600",
            "absolute bottom-full right-0 z-20 mb-2",
        )
    } else {
        (
            "inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:bg-slate-300 dark:disabled:bg-slate-700",
            "absolute right-0 top-full z-20 mt-2",
        )
    };
    let read_only_text = t!("upload-read-only");

    let mut close_and_refocus = move || {
        open.set(false);
        if let Some(button) = trigger() {
            spawn(async move {
                let _ = button.set_focus(true).await;
            });
        }
    };
    let mut pick = move |picker| {
        open.set(false);
        on_pick.call(picker);
    };

    rsx! {
        div {
            class: "relative",
            onkeydown: move |event| {
                if open() && event.key() == Key::Escape {
                    close_and_refocus();
                }
            },
            button {
                r#type: "button",
                class: button_class,
                disabled: read_only,
                title: if read_only { "{read_only_text}" },
                aria_describedby: if read_only { READ_ONLY_ID },
                aria_label: if floating { t!("upload-open") },
                aria_haspopup: "menu",
                aria_expanded: open(),
                aria_controls: menu_id,
                onmounted: move |event| trigger.set(Some(event.data())),
                onclick: move |_| open.toggle(),
                if floating {
                    PlusIcon {}
                } else {
                    ArrowUpIcon {}
                    {t!("upload")}
                }
            }
            if open() {
                // Transparent backdrop: a click anywhere outside the menu closes it.
                div {
                    class: "fixed inset-0 z-10",
                    onclick: move |_| open.set(false),
                }
                div {
                    id: menu_id,
                    role: "menu",
                    class: "{menu_class} w-44 rounded-lg border border-slate-200 bg-white p-1 shadow-lg dark:border-slate-700 dark:bg-slate-900",
                    button {
                        r#type: "button",
                        role: "menuitem",
                        class: MENU_ITEM,
                        // Focus moves into the menu so Escape reaches it in every browser.
                        onmounted: move |event| async move {
                            let _ = event.data().set_focus(true).await;
                        },
                        onclick: move |_| pick(Picker::Files),
                        {t!("upload-files")}
                    }
                    button {
                        r#type: "button",
                        role: "menuitem",
                        class: MENU_ITEM,
                        onclick: move |_| pick(Picker::Folder),
                        {t!("upload-folder")}
                    }
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn element(input: &MountedData) -> Option<&web_sys::Element> {
    input.downcast::<web_sys::Element>()
}

#[cfg(target_arch = "wasm32")]
fn click(input: &MountedData) {
    use wasm_bindgen::JsCast as _;

    if let Some(input) = element(input).and_then(|e| e.dyn_ref::<web_sys::HtmlElement>()) {
        input.click();
    }
}

/// Starts uploading what was picked, then clears the input so the same files can be picked again.
#[cfg(target_arch = "wasm32")]
fn picked(
    uploads: Uploads,
    volume: &str,
    path: &str,
    event: &FormEvent,
    input: Option<&MountedData>,
) {
    use wasm_bindgen::JsCast as _;

    let files: Vec<web_sys::File> = event
        .files()
        .iter()
        .filter_map(|file| file.inner().downcast_ref::<web_sys::File>().cloned())
        .collect();
    if let Some(input) = input
        .and_then(element)
        .and_then(|e| e.dyn_ref::<web_sys::HtmlInputElement>())
    {
        input.set_value("");
    }
    if !files.is_empty() {
        uploads.start(volume, path, files, Vec::new());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn click(_: &MountedData) {}

#[cfg(not(target_arch = "wasm32"))]
fn picked(_: Uploads, _: &str, _: &str, _: &FormEvent, _: Option<&MountedData>) {}
