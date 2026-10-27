use dioxus::prelude::*;

const DIALOG: &str = "m-auto w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 text-left shadow-xl backdrop:bg-slate-900/40 dark:border-slate-800 dark:bg-slate-900";

/// A native modal `<dialog>`: the browser traps focus, inerts the page and restores focus on close.
#[component]
pub fn Modal(labelled_by: String, on_close: EventHandler<()>, children: Element) -> Element {
    rsx! {
        dialog {
            aria_labelledby: labelled_by,
            class: DIALOG,
            onmounted: move |event| show_modal(&event.data()),
            oncancel: move |event| {
                // The parent unmounts the dialog; the browser must not close it behind its back.
                event.prevent_default();
                on_close.call(());
            },
            {children}
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_modal(dialog: &MountedData) {
    use wasm_bindgen::JsCast as _;

    if let Some(dialog) = dialog
        .downcast::<web_sys::Element>()
        .and_then(|e| e.dyn_ref::<web_sys::HtmlDialogElement>())
        && !dialog.open()
    {
        let _ = dialog.show_modal();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn show_modal(_: &MountedData) {}
