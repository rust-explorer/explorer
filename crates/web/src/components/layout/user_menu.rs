use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::auth::Me;

use super::SignOutButton;
use crate::components::common::LanguagePicker;

const LANGUAGE_SELECT_ID: &str = "user-menu-language";
const PANEL_ID: &str = "user-menu-panel";

/// Avatar letters: the first letter of the first and last words of `name`.
pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let Some(first) = words.next() else {
        return "?".to_owned();
    };
    let initial = |word: &str| word.chars().next().into_iter().flat_map(char::to_uppercase);
    let mut letters: String = initial(first).collect();
    if let Some(last) = words.last() {
        letters.extend(initial(last));
    }
    letters
}

#[component]
pub fn UserMenu(user: Me) -> Element {
    let mut open = use_signal(|| false);
    let mut avatar = use_signal(|| None::<Rc<MountedData>>);

    let mut close_and_refocus = move || {
        open.set(false);
        if let Some(button) = avatar() {
            spawn(async move {
                let _ = button.set_focus(true).await;
            });
        }
    };

    rsx! {
        div {
            class: "relative",
            onkeydown: move |event| {
                if open() && event.key() == Key::Escape {
                    close_and_refocus();
                }
            },
            button {
                r#type: "button",
                class: "inline-flex size-8 items-center justify-center rounded-full bg-indigo-200 text-xs font-bold text-indigo-800 hover:bg-indigo-300 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:bg-indigo-500/30 dark:text-indigo-100 dark:hover:bg-indigo-500/40",
                aria_label: t!("user-menu-open"),
                aria_haspopup: "dialog",
                aria_expanded: open(),
                aria_controls: if open() { PANEL_ID },
                onmounted: move |event| avatar.set(Some(event.data())),
                onclick: move |_| open.toggle(),
                {initials(&user.name)}
            }
            if open() {
                // Transparent backdrop: a click anywhere outside the panel closes it.
                div {
                    class: "fixed inset-0 z-10",
                    onclick: move |_| open.set(false),
                }
                div {
                    id: PANEL_ID,
                    tabindex: "-1",
                    class: "absolute right-0 top-full z-20 mt-2 w-64 rounded-lg border border-slate-200 bg-white p-3 shadow-lg focus:outline-none dark:border-slate-700 dark:bg-slate-900",
                    // Safari and macOS Firefox don't focus a clicked button, so Escape would go to <body>.
                    onmounted: move |event| async move {
                        let _ = event.data().set_focus(true).await;
                    },
                    p { class: "truncate text-sm font-semibold text-gray-900 dark:text-white", {user.name.clone()} }
                    if let Some(email) = &user.email {
                        p { class: "truncate text-xs text-slate-500 dark:text-slate-400", {email.clone()} }
                    }
                    div { class: "mt-3 border-t border-slate-100 pt-3 dark:border-slate-800",
                        LanguagePicker { id: LANGUAGE_SELECT_ID }
                    }
                    div { class: "mt-3 border-t border-slate-100 pt-3 dark:border-slate-800",
                        div { class: "-mx-2",
                            SignOutButton { class: "w-full rounded-md px-2 py-1.5 text-left text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-60 dark:text-slate-200 dark:hover:bg-slate-800" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_first_and_last_words() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ada king lovelace"), "AL");
        assert_eq!(initials("  Grace   Hopper "), "GH");
    }

    #[test]
    fn initials_of_a_single_word_or_empty_name() {
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("élodie"), "É");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("   "), "?");
    }
}
