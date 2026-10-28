use std::cell::Cell;

use dioxus::prelude::*;
use dioxus_i18n::t;

use super::Modal;
use crate::conflict::Decision;

const BUTTON: &str = "rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";
const PRIMARY: &str = "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600";

/// Asks what to do with an item whose destination already exists; the bool is "apply to all".
/// Callers key it per item so the next prompt starts unchecked.
#[component]
pub fn ConflictPrompt(
    name: String,
    folder: String,
    several: bool,
    allow_replace: bool,
    on_decide: EventHandler<(Decision, bool)>,
) -> Element {
    let mut apply_to_all = use_signal(|| false);
    // A rename's prompt can open over an upload's: each prompt gets its own ids.
    let (title_id, apply_all_id) = use_hook(|| {
        let n = next_instance();
        (
            format!("conflict-title-{n}"),
            format!("conflict-apply-all-{n}"),
        )
    });
    let decide = move |decision| on_decide.call((decision, apply_to_all()));

    rsx! {
        Modal { labelled_by: title_id.clone(), on_close: move |()| decide(Decision::Cancel),
            h2 {
                id: title_id,
                class: "break-words text-sm font-medium text-gray-900 dark:text-white",
                {t!("conflict-title", name: name, folder: folder)}
            }
            if several {
                div { class: "mt-4 flex items-center gap-2",
                    input {
                        id: apply_all_id.clone(),
                        r#type: "checkbox",
                        class: "size-4 rounded border-slate-300 text-indigo-600",
                        checked: apply_to_all(),
                        onchange: move |event| apply_to_all.set(event.checked()),
                    }
                    label {
                        r#for: apply_all_id,
                        class: "text-sm text-slate-700 dark:text-slate-300",
                        {t!("conflict-apply-all")}
                    }
                }
            }
            div { class: "mt-5 flex flex-wrap justify-end gap-3",
                button {
                    r#type: "button",
                    class: BUTTON,
                    onclick: move |_| decide(Decision::Cancel),
                    {t!("cancel")}
                }
                if allow_replace {
                    button {
                        r#type: "button",
                        class: BUTTON,
                        onclick: move |_| decide(Decision::Replace),
                        {t!("conflict-replace")}
                    }
                }
                button {
                    r#type: "button",
                    class: PRIMARY,
                    autofocus: true,
                    onclick: move |_| decide(Decision::KeepBoth),
                    {t!("conflict-keep-both")}
                }
            }
        }
    }
}

thread_local! {
    static INSTANCES: Cell<u64> = const { Cell::new(0) };
}

fn next_instance() -> u64 {
    INSTANCES.with(|count| {
        count.set(count.get() + 1);
        count.get()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_are_numbered_apart() {
        assert_ne!(next_instance(), next_instance());
    }
}
