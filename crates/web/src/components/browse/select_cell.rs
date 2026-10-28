use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::i18n::attr;

const BOX: &str = "size-4 cursor-pointer rounded border-slate-300 accent-indigo-600 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-600";

// The selection owns the state. A click is not cancelled (the browser would revert the box after
// the next render); it puts the box back as last rendered and the next render shows the outcome.

/// A row's checkbox; a shift-click asks for a range instead of a toggle.
#[component]
pub fn SelectCell(name: String, checked: bool, on_select: EventHandler<bool>) -> Element {
    rsx! {
        div { role: "cell", class: "flex items-center justify-center self-stretch",
            SelectBox { name, checked, on_select }
        }
    }
}

/// An entry's checkbox, as in [`SelectCell`].
#[component]
pub fn SelectBox(name: String, checked: bool, on_select: EventHandler<bool>) -> Element {
    let mut input = use_signal(|| None::<Rc<MountedData>>);

    rsx! {
        input {
            r#type: "checkbox",
            class: BOX,
            aria_label: attr(t!("select-entry", name: name.clone())),
            checked,
            onmounted: move |event| input.set(Some(event.data())),
            onclick: move |event: MouseEvent| {
                if let Some(input) = input() {
                    show_state(&input, checked, false);
                }
                on_select.call(event.modifiers().shift());
            },
        }
    }
}

/// The header checkbox: everything, nothing, or partly (`indeterminate`). `input` holds the box,
/// for dialogs that hand focus back to it. Outside a table, `role` is `none`.
#[component]
pub fn SelectAll(
    mut input: Signal<Option<Rc<MountedData>>>,
    checked: bool,
    partial: bool,
    on_toggle: EventHandler<()>,
    #[props(default = "columnheader")] role: &'static str,
) -> Element {
    use_effect(use_reactive!(|checked, partial| {
        if let Some(input) = input() {
            show_state(&input, checked, partial);
        }
    }));

    rsx! {
        // As wide as the select column, so the box stays put when the selection bar replaces
        // the column headers.
        div { role, class: "flex w-6 shrink-0 items-center justify-center",
            input {
                r#type: "checkbox",
                class: BOX,
                aria_label: t!("select-all"),
                checked,
                onmounted: move |event| {
                    show_state(&event.data(), checked, partial);
                    input.set(Some(event.data()));
                },
                onclick: move |_| {
                    if let Some(input) = input() {
                        show_state(&input, checked, partial);
                    }
                    on_toggle.call(());
                },
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_state(input: &MountedData, checked: bool, indeterminate: bool) {
    use wasm_bindgen::JsCast as _;

    if let Some(input) = input
        .downcast::<web_sys::Element>()
        .and_then(|e| e.dyn_ref::<web_sys::HtmlInputElement>())
    {
        input.set_checked(checked);
        input.set_indeterminate(indeterminate);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn show_state(_: &MountedData, _: bool, _: bool) {}
