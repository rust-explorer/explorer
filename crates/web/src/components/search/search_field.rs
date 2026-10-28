use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::common::icons::{ArrowLeftIcon, SearchIcon};
use crate::search::{DEBOUNCE_MS, SearchQuery, sleep};

const INPUT_CLASS: &str = "h-10 min-w-0 flex-1 rounded-md border border-slate-300 bg-white px-3 text-base text-slate-900 placeholder:text-slate-500 focus-visible:outline-2 focus-visible:outline-indigo-600 md:h-9 md:text-sm dark:border-slate-700 dark:bg-slate-950 dark:text-white dark:placeholder:text-slate-400";
const BUTTON_CLASS: &str = "inline-flex size-10 shrink-0 items-center justify-center rounded-md text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 md:hidden dark:text-slate-200 dark:hover:bg-slate-800";

/// Searches the volume by name. From `md` it is an input in the header; below, a magnifier opens
/// the same input over the header. Typing sets the shared [`SearchQuery`] after a pause.
#[component]
pub fn SearchField(volume: String) -> Element {
    let SearchQuery(mut query) = use_context();
    let mut text = use_signal(|| query.peek().clone());
    let mut open = use_signal(|| false);
    let mut generation = use_signal(|| 0_u64);
    let mut input = use_signal(|| None::<Rc<MountedData>>);

    // Navigation clears the query: the field follows, and a pending keystroke is dropped. Spaces
    // alone are no query and stay as typed.
    use_effect(move || {
        if query.read().is_empty() && !text.peek().trim().is_empty() {
            generation.with_mut(|generation| *generation += 1);
            text.set(String::new());
        }
    });

    use_effect(move || {
        if open()
            && let Some(input) = input.peek().clone()
        {
            spawn(async move {
                let _ = input.set_focus(true).await;
            });
        }
    });

    let mut apply = move |value: &str| {
        generation.with_mut(|generation| *generation += 1);
        let value = value.trim();
        if *query.peek() != value {
            query.set(value.to_owned());
        }
    };
    let mut clear = move || {
        text.set(String::new());
        apply("");
    };

    let placeholder = t!("search-placeholder", volume: volume);
    let wrapper = if open() {
        "absolute inset-0 z-10 flex items-center gap-2 bg-white px-2 md:static md:inset-auto md:z-auto md:w-64 md:bg-transparent md:px-0 dark:bg-slate-900"
    } else {
        "hidden md:flex md:w-64 md:items-center"
    };

    rsx! {
        if !open() {
            button {
                r#type: "button",
                class: BUTTON_CLASS,
                aria_label: t!("search-open"),
                onclick: move |_| open.set(true),
                SearchIcon {}
            }
        }
        div { class: wrapper, role: "search",
            button {
                r#type: "button",
                class: BUTTON_CLASS,
                aria_label: t!("search-close"),
                onclick: move |_| {
                    clear();
                    open.set(false);
                },
                ArrowLeftIcon {}
            }
            input {
                r#type: "search",
                class: INPUT_CLASS,
                value: "{text}",
                placeholder: placeholder.clone(),
                aria_label: placeholder,
                autocomplete: "off",
                spellcheck: "false",
                onmounted: move |event| input.set(Some(event.data())),
                oninput: move |event| {
                    let value = event.value();
                    text.set(value.clone());
                    if value.trim().is_empty() {
                        apply("");
                        return;
                    }
                    let id = generation.with_mut(|generation| {
                        *generation += 1;
                        *generation
                    });
                    spawn(async move {
                        sleep(DEBOUNCE_MS).await;
                        if *generation.peek() == id {
                            apply(&value);
                        }
                    });
                },
                onkeydown: move |event| match event.key() {
                    Key::Escape => {
                        clear();
                        open.set(false);
                    }
                    Key::Enter => apply(&text.peek().clone()),
                    _ => {}
                },
            }
        }
    }
}
