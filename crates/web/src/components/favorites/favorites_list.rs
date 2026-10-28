use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::favorites::Favorite;
use shared::api::tasks::Location;

use crate::components::common::file_name;
use crate::components::common::icons::CloseIcon;
use crate::favorites::use_favorites;
use crate::routes::{Route, Segments};

/// The Favorites section of the sidebar; absent while there are none.
#[component]
pub fn FavoritesList() -> Element {
    let favorites = use_favorites();
    let list = favorites.list()().unwrap_or_default();
    let load_failed = favorites.load_failed();
    if list.is_empty() && !load_failed {
        return rsx! {};
    }

    rsx! {
        section { class: "mt-3 flex flex-col gap-1", aria_label: t!("favorites-title"),
            h2 { class: "mx-1 mb-1 mt-1.5 text-[11px] font-medium uppercase tracking-wider text-slate-400 dark:text-slate-500",
                {t!("favorites-title")}
            }
            if load_failed {
                div { role: "alert", class: "mx-1 flex flex-wrap items-center gap-x-2 text-xs text-slate-600 dark:text-slate-400",
                    span { {t!("favorites-load-failed")} }
                    button {
                        r#type: "button",
                        class: "rounded text-indigo-600 hover:underline focus-visible:outline-2 focus-visible:outline-indigo-600 dark:text-indigo-400",
                        onclick: move |_| favorites.reload(),
                        {t!("try-again")}
                    }
                }
            }
            ul { class: "flex flex-col gap-0.5",
                for favorite in list {
                    li { key: "{favorite.volume}/{favorite.path}", FavoriteRow { favorite } }
                }
            }
        }
    }
}

/// The name shown for a favorite: the folder's, or the volume's for a root.
fn display_name(favorite: &Favorite) -> &str {
    if favorite.path.is_empty() {
        &favorite.volume
    } else {
        file_name(&favorite.path)
    }
}

fn full_name(favorite: &Favorite) -> String {
    if favorite.path.is_empty() {
        favorite.volume.clone()
    } else {
        format!("{}/{}", favorite.volume, favorite.path)
    }
}

fn segments(path: &str) -> Segments {
    path.split('/')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into()
}

#[component]
fn FavoriteRow(favorite: Favorite) -> Element {
    let store = use_favorites();
    let name = display_name(&favorite).to_owned();
    let title = full_name(&favorite);
    let remove_label = t!("favorites-remove", name: name.clone());
    let location = Location {
        volume: favorite.volume.clone(),
        path: favorite.path.clone(),
    };
    let body = rsx! {
        span { class: "min-w-0 flex-1",
            span { class: "block truncate text-sm", {name.clone()} }
            span { class: "block truncate text-[11px] text-slate-400 dark:text-slate-500",
                if favorite.exists {
                    {favorite.volume.clone()}
                } else {
                    {t!("favorites-not-found")}
                }
            }
        }
    };

    rsx! {
        div { class: "flex items-center gap-0.5",
            if favorite.exists {
                Link {
                    to: Route::Browse {
                        volume: favorite.volume.clone().into(),
                        path: segments(&favorite.path),
                    },
                    class: "flex min-h-10 min-w-0 flex-1 items-center rounded-md px-2 py-1 text-slate-700 hover:bg-slate-100 md:min-h-9 dark:text-slate-300 dark:hover:bg-slate-800",
                    title: "{title}",
                    {body}
                }
            } else {
                div {
                    class: "flex min-h-10 min-w-0 flex-1 items-center rounded-md px-2 py-1 text-slate-400 md:min-h-9 dark:text-slate-500",
                    title: "{title}",
                    {body}
                }
            }
            button {
                r#type: "button",
                class: "inline-flex size-10 shrink-0 items-center justify-center rounded-md text-slate-400 hover:bg-slate-100 hover:text-slate-600 focus-visible:outline-2 focus-visible:outline-indigo-600 md:size-7 dark:hover:bg-slate-800 dark:hover:text-slate-300",
                aria_label: "{remove_label}",
                title: "{remove_label}",
                onclick: move |_| store.remove(location.clone()),
                CloseIcon { class: "size-3.5" }
            }
        }
    }
}
