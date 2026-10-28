use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

const DIALOG: &str = "m-auto w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 text-left shadow-xl backdrop:bg-slate-900/40 dark:border-slate-800 dark:bg-slate-900";

/// A native modal `<dialog>`: the browser traps focus and inerts the page; focus returns to the opener on unmount.
/// A dialog opened from a menu item gets `return_focus`: the item that had focus is gone by then.
#[component]
pub fn Modal(
    labelled_by: String,
    on_close: EventHandler<()>,
    #[props(default)] return_focus: Option<FocusTarget>,
    children: Element,
) -> Element {
    let opened = use_hook(|| Rc::new(RefCell::new(Opened::default())));
    let focus_target = use_hook(|| Rc::new(RefCell::new(None::<FocusTarget>)));
    focus_target.replace(return_focus);
    use_drop({
        let opened = opened.clone();
        move || {
            let target = focus_target.borrow();
            restore(&opened.borrow(), target.as_ref().map(|t| &*t.0));
        }
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

/// An element to hand focus to, compared by identity so that it can be a prop.
#[derive(Clone)]
pub struct FocusTarget(pub Rc<MountedData>);

impl PartialEq for FocusTarget {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
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
fn restore(opened: &Opened, return_focus: Option<&MountedData>) {
    use wasm_bindgen::JsCast as _;

    if let Some(dialog) = opened.dialog.as_ref().filter(|d| d.open()) {
        dialog.close();
    }
    let target = match return_focus {
        Some(target) => target
            .downcast::<web_sys::Element>()
            .and_then(|e| e.dyn_ref::<web_sys::HtmlElement>()),
        None => opened.previous.as_ref(),
    };
    if let Some(target) = target.filter(|t| t.is_connected()) {
        let _ = target.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn show_modal(_: &MountedData, _: &mut Opened) {}

#[cfg(not(target_arch = "wasm32"))]
fn restore(_: &Opened, _: Option<&MountedData>) {}
