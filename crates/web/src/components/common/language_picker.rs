use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;

use crate::i18n::{change_locale, native_name, supported};

/// Labelled language select; `id` ties the label to the select, so it must be unique on the page.
#[component]
pub fn LanguagePicker(id: &'static str) -> Element {
    let current = i18n().language();
    rsx! {
        label {
            r#for: id,
            class: "block text-xs font-medium text-slate-500 dark:text-slate-400",
            {t!("language")}
        }
        select {
            id,
            class: "mt-1 w-full rounded-md border border-slate-300 bg-white px-2 py-1 text-sm dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100",
            onchange: move |event| {
                if let Ok(locale) = event.value().parse() {
                    change_locale(i18n(), locale);
                }
            },
            for locale in supported() {
                option {
                    value: "{locale}",
                    lang: "{locale}",
                    selected: locale == current,
                    {native_name(&locale)}
                }
            }
        }
    }
}
