use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::common::icons::RefreshIcon;
use crate::revision::Revision;

#[component]
pub fn RefreshButton(busy: bool) -> Element {
    let Revision(mut revision) = use_context();
    let icon_class = if busy {
        "size-5 animate-spin motion-reduce:animate-none"
    } else {
        "size-5"
    };
    rsx! {
        button {
            r#type: "button",
            class: "inline-flex size-10 items-center justify-center rounded-md text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 aria-disabled:cursor-default aria-disabled:opacity-60 dark:text-slate-200 dark:hover:bg-slate-800",
            aria_label: t!("refresh"),
            title: t!("refresh"),
            aria_disabled: busy,
            onclick: move |_| {
                if !busy {
                    *revision.write() += 1;
                }
            },
            RefreshIcon { class: icon_class }
        }
    }
}
