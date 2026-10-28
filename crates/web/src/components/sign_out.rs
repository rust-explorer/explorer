use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::api;

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Idle,
    Pending,
    Failed,
}

#[component]
pub fn SignOutButton(class: String) -> Element {
    let mut status = use_signal(|| Status::Idle);

    rsx! {
        button {
            r#type: "button",
            class,
            disabled: status() == Status::Pending,
            onclick: move |_| async move {
                status.set(Status::Pending);
                // On success the browser is already leaving the page.
                if api::sign_out().await.is_err() {
                    status.set(Status::Failed);
                }
            },
            {t!("sign-out")}
        }
        if status() == Status::Failed {
            p { role: "alert", class: "text-sm text-red-600 dark:text-red-400", {t!("sign-out-failed")} }
        }
    }
}
