use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::NewFolderDialog;
use crate::components::common::icons::{ArrowUpIcon, PlusIcon};
use crate::components::common::{FocusTarget, Menu, MenuItem};
use crate::uploads::{Uploads, use_uploads};

/// An item of the Upload menu.
#[derive(Clone, Copy, PartialEq)]
enum AddChoice {
    Files,
    Folder,
    NewFolder,
}

/// The header's Upload button, a floating "+" below `md`, and the file pickers behind them.
/// A read-only volume gets none of them: the browse page's notice says why.
#[component]
pub fn UploadButton(volume: String, path: String, read_only: bool) -> Element {
    let uploads = use_uploads();
    let mut files_input = use_signal(|| None::<Rc<MountedData>>);
    let mut folder_input = use_signal(|| None::<Rc<MountedData>>);
    let mut new_folder = use_signal(|| false);
    let mut floating_trigger = use_signal(|| None::<FocusTarget>);
    let (dialog_volume, dialog_path) = (volume.clone(), path.clone());

    let choose = move |choice| {
        let input = match choice {
            AddChoice::Files => files_input(),
            AddChoice::Folder => folder_input(),
            AddChoice::NewFolder => {
                new_folder.set(true);
                return;
            }
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

    if read_only {
        return rsx! {};
    }

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
        div { class: "hidden md:block",
            UploadMenu { floating: false, on_choose: choose, on_trigger: |_| {} }
        }
        div { class: "fixed bottom-4 right-4 z-30 md:hidden",
            UploadMenu {
                floating: true,
                on_choose: choose,
                on_trigger: move |trigger| floating_trigger.set(Some(trigger)),
            }
        }
        if new_folder() {
            NewFolderDialog {
                volume: dialog_volume,
                path: dialog_path,
                return_focus: floating_trigger(),
                on_close: move |()| new_folder.set(false),
            }
        }
    }
}

/// `on_trigger` gets the menu's button, which a dialog opened from the menu hands focus back to.
#[component]
fn UploadMenu(
    floating: bool,
    on_choose: EventHandler<AddChoice>,
    on_trigger: EventHandler<FocusTarget>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut trigger = use_signal(|| None::<Rc<MountedData>>);
    let menu_id = if floating {
        "upload-menu-floating"
    } else {
        "upload-menu"
    };
    let (button_class, menu_class) = if floating {
        (
            "inline-flex size-14 items-center justify-center rounded-full bg-indigo-600 text-white shadow-lg hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600",
            "absolute bottom-full right-0 mb-2",
        )
    } else {
        (
            "inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600",
            "absolute right-0 top-full mt-2",
        )
    };

    let close = move |restore_focus: bool| {
        open.set(false);
        if restore_focus && let Some(button) = trigger() {
            spawn(async move {
                let _ = button.set_focus(true).await;
            });
        }
    };
    let mut choose = move |choice| {
        open.set(false);
        on_choose.call(choice);
    };

    rsx! {
        div { class: "relative",
            button {
                r#type: "button",
                class: button_class,
                aria_label: if floating { t!("upload-open") },
                aria_haspopup: "menu",
                aria_expanded: open(),
                aria_controls: menu_id,
                onmounted: move |event| {
                    on_trigger.call(FocusTarget(event.data()));
                    trigger.set(Some(event.data()));
                },
                onclick: move |_| open.toggle(),
                if floating {
                    PlusIcon {}
                } else {
                    ArrowUpIcon {}
                    {t!("upload")}
                }
            }
            if open() {
                Menu {
                    id: menu_id,
                    class: "{menu_class} z-20",
                    label: t!("upload-open"),
                    on_close: close,
                    MenuItem { on_select: move |()| choose(AddChoice::Files), {t!("upload-files")} }
                    MenuItem { on_select: move |()| choose(AddChoice::Folder), {t!("upload-folder")} }
                    if floating {
                        MenuItem { on_select: move |()| choose(AddChoice::NewFolder), {t!("new-folder")} }
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
