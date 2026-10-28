use dioxus::prelude::*;

#[component]
pub fn Notice(title: String, message: String, children: Element) -> Element {
    rsx! {
        section { class: "w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 shadow-sm dark:border-slate-800 dark:bg-slate-900",
            h1 { class: "text-lg font-semibold text-gray-900 dark:text-white", {title} }
            p { class: "mt-2 text-sm text-slate-600 dark:text-slate-400", {message} }
            div { class: "mt-6 flex flex-wrap items-center gap-3", {children} }
        }
    }
}
