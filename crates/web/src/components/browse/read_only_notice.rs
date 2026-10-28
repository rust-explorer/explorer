use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::common::icons::LockIcon;

/// Under the breadcrumb of a read-only volume: the short text on phones, the full one from `md`.
#[component]
pub fn ReadOnlyNotice() -> Element {
    let full = t!("read-only-notice");

    rsx! {
        p {
            role: "note",
            title: "{full}",
            class: "flex items-center gap-2 rounded-md border border-slate-200 bg-slate-50 px-3 py-1.5 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-400",
            LockIcon { class: "size-4 shrink-0" }
            span { class: "truncate md:hidden", {t!("read-only-notice-short")} }
            span { class: "hidden md:inline", {full.clone()} }
        }
    }
}
