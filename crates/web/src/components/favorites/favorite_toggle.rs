use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::tasks::Location;

use crate::components::common::error_message;
use crate::components::common::icons::StarIcon;
use crate::favorites::use_favorites;

/// Star button in the breadcrumb bar: adds or removes the current folder.
#[component]
pub fn FavoriteToggle(volume: String, path: String) -> Element {
    let favorites = use_favorites();
    let pressed = favorites.contains(&volume, &path);
    let label = if pressed {
        t!("favorites-remove-current")
    } else {
        t!("favorites-add")
    };
    let location = Location {
        volume: volume.clone(),
        path: path.clone(),
    };

    // A failure belongs to the folder it happened on.
    use_effect(use_reactive!(|volume, path| {
        let _ = (volume, path);
        favorites.clear_failure();
    }));

    let failure = favorites.failure()();
    let (color, hover) = if pressed {
        (
            "text-amber-500 dark:text-amber-400",
            "hover:bg-amber-50 dark:hover:bg-slate-800",
        )
    } else {
        (
            "text-slate-500 dark:text-slate-400",
            "hover:bg-slate-100 dark:hover:bg-slate-800",
        )
    };

    rsx! {
        button {
            r#type: "button",
            class: "inline-flex size-10 shrink-0 items-center justify-center rounded-md {color} {hover} focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 md:size-8",
            aria_label: "{label}",
            title: "{label}",
            aria_pressed: "{pressed}",
            onclick: move |_| {
                if pressed {
                    favorites.remove(location.clone());
                } else {
                    favorites.add(location.clone());
                }
            },
            StarIcon { filled: pressed }
        }
        if let Some(failure) = failure {
            p { role: "alert", class: "basis-full text-sm text-red-600 dark:text-red-400",
                {error_message(&failure)}
            }
        }
    }
}
