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
                p { class: "mt-1", {t!("browse-empty-hint")} }
            }
        }
    }
}
