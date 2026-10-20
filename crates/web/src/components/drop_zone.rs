use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::uploads::use_uploads;

/// The browse page's content, with an overlay while files are dragged over it, and the drop target.
/// `read_only` is `None` until the volume is known; nothing is accepted before that.
#[component]
pub fn DropZone(
    volume: String,
    path: String,
    read_only: Option<bool>,
    children: Element,
) -> Element {
    let uploads = use_uploads();
    // Enters minus leaves: moving between child elements fires both.
    let mut depth = use_signal(|| 0_u32);
    let accepts = read_only == Some(false);

    let folder = path.rsplit('/').next().filter(|name| !name.is_empty());
    let folder = folder.unwrap_or(&volume).to_owned();

    rsx! {
        div {
            class: "relative flex h-full min-h-0 flex-col gap-4",
            ondragenter: move |event| {
                if carries_files(&event) {
                    event.prevent_default();
                    depth += 1;
                }
            },
            ondragover: move |event| {
                if carries_files(&event) {
                    event.prevent_default();
                    set_drop_effect(&event, accepts);
                }
            },
            ondragleave: move |event| {
                if carries_files(&event) {
                    depth.set(depth().saturating_sub(1));
                }
            },
            ondrop: move |event| {
                if !carries_files(&event) {
                    return;
                }
                event.prevent_default();
                depth.set(0);
                if accepts {
                    start(uploads, &volume, &path, &event);
                }
            },
            {children}
            if depth() > 0 && read_only.is_some() {
                div {
                    class: "pointer-events-none absolute inset-0 z-20 flex items-center justify-center rounded-lg border-2 border-dashed border-indigo-500 bg-indigo-50/90 p-6 text-center text-base font-medium text-indigo-700 dark:border-indigo-400 dark:bg-indigo-950/90 dark:text-indigo-200",
                    if accepts {
                        {t!("upload-drop", folder: folder)}
                    } else {
                        {t!("upload-read-only")}
                    }
                }
            }
        }
    }
}

/// Dragged rows and text carry no `Files`, and show no overlay.
#[cfg(target_arch = "wasm32")]
fn carries_files(event: &DragEvent) -> bool {
    event
        .data()
        .downcast::<web_sys::DragEvent>()
        .and_then(web_sys::DragEvent::data_transfer)
        .is_some_and(|data| data.types().iter().any(|kind| kind == "Files"))
}

#[cfg(target_arch = "wasm32")]
fn set_drop_effect(event: &DragEvent, accepts: bool) {
    if let Some(data) = event
        .data()
        .downcast::<web_sys::DragEvent>()
        .and_then(web_sys::DragEvent::data_transfer)
    {
        data.set_drop_effect(if accepts { "copy" } else { "none" });
    }
}

#[cfg(target_arch = "wasm32")]
fn start(uploads: crate::uploads::Uploads, volume: &str, path: &str, event: &DragEvent) {
    let Some(data) = event
        .data()
        .downcast::<web_sys::DragEvent>()
        .and_then(web_sys::DragEvent::data_transfer)
    else {
        return;
    };
    // The items are read now; the walk of dropped folders finishes later.
    let collected = crate::uploads::drop::collect(&data);
    let (volume, path) = (volume.to_owned(), path.to_owned());
    dioxus::core::spawn_forever(async move {
        let (files, dirs) = collected.await;
        if !files.is_empty() || !dirs.is_empty() {
            uploads.start_relative(&volume, &path, files, dirs);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn carries_files(_: &DragEvent) -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn set_drop_effect(_: &DragEvent, _: bool) {}

#[cfg(not(target_arch = "wasm32"))]
fn start(_: crate::uploads::Uploads, _: &str, _: &str, _: &DragEvent) {}
