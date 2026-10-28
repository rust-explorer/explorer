use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;

const DIALOG: &str = "m-auto w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 text-left shadow-xl dark:border-slate-800 dark:bg-slate-900";

/// A native modal `<dialog>`: the browser traps focus and inerts the page; focus returns to the opener on unmount.
/// A dialog opened from a menu item gets `return_focus`: the item that had focus is gone by then.
/// `on_close` runs once, on Escape or when the browser closes the dialog by itself; the parent
/// then unmounts it.
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
    let close = use_hook(|| Rc::new(Close::default()));
    close.handler.set(Some(on_close));
    use_drop({
        let opened = opened.clone();
        let close = close.clone();
        move || {
            close.done.set(true);
            let target = focus_target.borrow();
            restore(&mut opened.borrow_mut(), target.as_ref().map(|t| &*t.0));
        }
    });
    let on_mounted = {
        let close = close.clone();
        move |event: MountedEvent| {
            let close = close.clone();
            show_modal(&event.data(), &mut opened.borrow_mut(), move || {
                close.fire();
            });
        }
    };

    rsx! {
        dialog {
            aria_labelledby: labelled_by,
            class: DIALOG,
            onmounted: on_mounted,
            oncancel: move |event| {
                // The parent unmounts the dialog; the browser must not close it behind its back.
                event.prevent_default();
                close.fire();
            },
            {children}
        }
    }
}

/// The parent's latest `on_close`, and whether it already ran or the dialog is gone.
#[derive(Default)]
struct Close {
    handler: Cell<Option<EventHandler<()>>>,
    done: Cell<bool>,
}

impl Close {
    fn fire(&self) {
        if !self.done.replace(true)
            && let Some(handler) = self.handler.get()
        {
            handler.call(());
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

/// The open dialog, the element that had focus before it, to hand focus back on unmount, and
/// the listener told when the browser closes the dialog without a `cancel` event (a repeated
/// Escape, for instance).
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Opened {
    dialog: Option<web_sys::HtmlDialogElement>,
    previous: Option<web_sys::HtmlElement>,
    on_close: Option<wasm_bindgen::closure::Closure<dyn Fn()>>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Opened {}

#[cfg(target_arch = "wasm32")]
fn show_modal(dialog: &MountedData, opened: &mut Opened, on_close: impl Fn() + 'static) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

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
            let on_close = Closure::<dyn Fn()>::new(on_close);
            let _ =
                dialog.add_event_listener_with_callback("close", on_close.as_ref().unchecked_ref());
            opened.on_close = Some(on_close);
            opened.dialog = Some(dialog.clone());
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn restore(opened: &mut Opened, return_focus: Option<&MountedData>) {
    use wasm_bindgen::JsCast as _;

    if let Some(dialog) = opened.dialog.as_ref() {
        if let Some(on_close) = opened.on_close.take() {
            let _ = dialog
                .remove_event_listener_with_callback("close", on_close.as_ref().unchecked_ref());
        }
        if dialog.open() {
            dialog.close();
        }
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
fn show_modal(_: &MountedData, _: &mut Opened, _: impl Fn() + 'static) {}

#[cfg(not(target_arch = "wasm32"))]
fn restore(_: &mut Opened, _: Option<&MountedData>) {}
