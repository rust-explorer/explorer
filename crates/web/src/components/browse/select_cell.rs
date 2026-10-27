use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;

const BOX: &str = "size-4 cursor-pointer rounded border-slate-300 accent-indigo-600 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-600";

/// A row's checkbox; a shift-click asks for a range instead of a toggle.
#[component]
pub fn SelectCell(name: String, checked: bool, on_select: EventHandler<bool>) -> Element {
    rsx! {
        div { role: "cell", class: "flex items-center justify-center self-stretch",
            input {
                r#type: "checkbox",
                class: BOX,
                aria_label: t!("select-entry", name: name.clone()),
                checked,
                onclick: move |event: MouseEvent| {
                    // The state lives in the selection, which also answers a range click.
                    event.prevent_default();
                    on_select.call(event.modifiers().shift());
                },
            }
        }
    }
}

/// The header checkbox: everything, nothing, or partly (`indeterminate`).
#[component]
pub fn SelectAll(checked: bool, partial: bool, on_toggle: EventHandler<()>) -> Element {
    let mut input = use_signal(|| None::<Rc<MountedData>>);
    use_effect(use_reactive!(|partial| {
        if let Some(input) = input() {
            set_indeterminate(&input, partial);
        }
    }));

    rsx! {
        div { role: "columnheader", class: "flex items-center justify-center",
            input {
                r#type: "checkbox",
                class: BOX,
                aria_label: t!("select-all"),
                checked,
                onmounted: move |event| input.set(Some(event.data())),
                onclick: move |event: MouseEvent| {
                    event.prevent_default();
                    on_toggle.call(());
                },
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn set_indeterminate(input: &MountedData, value: bool) {
    use wasm_bindgen::JsCast as _;

    if let Some(input) = input
        .downcast::<web_sys::Element>()
        .and_then(|e| e.dyn_ref::<web_sys::HtmlInputElement>())
    {
        input.set_indeterminate(value);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn set_indeterminate(_: &MountedData, _: bool) {}
