use dioxus::prelude::*;
use dioxus_i18n::t;

use super::file_row::MUTED;

/// The empty state of a folder; the upload hint only where uploading is possible.
#[component]
pub fn EmptyFolder(read_only: bool) -> Element {
    rsx! {
        div { class: "py-10 text-center {MUTED}",
            p { {t!("browse-empty")} }
            if !read_only {
                // Phones have the floating "+" instead of the Upload button.
                p { class: "mt-1 hidden md:block", {t!("browse-empty-hint")} }
                p { class: "mt-1 md:hidden", {t!("browse-empty-hint-touch")} }
            }
        }
    }
}
