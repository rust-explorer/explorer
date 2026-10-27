use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

const DIALOG: &str = "m-auto w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 text-left shadow-xl backdrop:bg-slate-900/40 dark:border-slate-800 dark:bg-slate-900";

/// A native modal `<dialog>`: the browser traps focus and inerts the page; focus returns to the opener on unmount.
#[component]
pub fn Modal(labelled_by: String, on_close: EventHandler<()>, children: Element) -> Element {
    let opened = use_hook(|| Rc::new(RefCell::new(Opened::default())));
    use_drop({
        let opened = opened.clone();
        move || restore(&opened.borrow())
    });

    rsx! {
        dialog {
            aria_labelledby: labelled_by,
            class: DIALOG,
            onmounted: move |event| show_modal(&event.data(), &mut opened.borrow_mut()),
            oncancel: move |event| {
                // The parent unmounts the dialog; the browser must not close it behind its back.
                event.prevent_default();
                on_close.call(());
            },
            {children}
        }
    }
}

/// The open dialog and the element that had focus before it, to hand focus back on unmount.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Opened {
    dialog: Option<web_sys::HtmlDialogElement>,
    previous: Option<web_sys::HtmlElement>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Opened {}

#[cfg(target_arch = "wasm32")]
fn show_modal(dialog: &MountedData, opened: &mut Opened) {
    use wasm_bindgen::JsCast as _;

    if let Some(dialog) = dialog
        .downcast::<web_sys::Element>()
        .and_then(|e| e.dyn_ref::<web_sys::HtmlDialogElement>())
        && !dialog.open()
    {
        opened.previous = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element())
            .and_then(|e| e.dyn_into().ok());
        if dialog.show_modal().is_ok() {
            opened.dialog = Some(dialog.clone());
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn restore(opened: &Opened) {
    if let Some(dialog) = opened.dialog.as_ref().filter(|d| d.open()) {
        dialog.close();
    }
    if let Some(previous) = opened.previous.as_ref().filter(|p| p.is_connected()) {
        let _ = previous.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn show_modal(_: &MountedData, _: &mut Opened) {}

#[cfg(not(target_arch = "wasm32"))]
fn restore(_: &Opened) {}
