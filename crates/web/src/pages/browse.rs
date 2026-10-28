use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::{Entry, EntryKind, Listing, Me, VolumeInfo};

use crate::api;
use crate::components::icons::{FileIcon, FolderIcon};
use crate::components::{Breadcrumb, ErrorPanel, Shell, Sidebar};
use crate::format::format_size;
use crate::routes::{Route, Segment, Segments};

const ROW_CLASS: &str = "flex items-center gap-3 px-3 py-2.5 text-sm";

#[component]
pub fn Browse(volume: Segment, path: Segments) -> Element {
    let mut me = use_resource(|| api::get_json::<Me>("/api/me"));
    let mut volumes = use_resource(|| api::get_json::<Vec<VolumeInfo>>("/api/volumes"));
    let url = api::entries_url(volume.as_str(), &path.joined());
    let mut listing = use_resource(use_reactive!(|url| async move {
        let result = api::get_json::<Listing>(&url).await;
        (url, result)
    }));

    let user = match me.read().clone() {
        Some(Ok(user)) => user,
        Some(Err(failure)) if !failure.is_unauthenticated() => {
            return rsx! {
                Shell { ErrorPanel { failure, on_retry: move |()| me.restart() } }
            };
        }
        // Still loading, or already redirecting to the sign-in page.
        _ => {
            return rsx! {
                Shell { p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} } }
            };
        }
    };

    let sidebar = match volumes.read().clone() {
        Some(Ok(volumes)) => rsx! {
            Sidebar { volumes, current: volume.as_str().to_owned() }
        },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            div { class: "flex flex-col items-start gap-2 px-1 text-sm",
                p { role: "alert", class: "text-slate-600 dark:text-slate-400", {t!("load-failed")} }
                button {
                    r#type: "button",
                    class: "text-indigo-600 hover:underline dark:text-indigo-400",
                    onclick: move |_| volumes.restart(),
                    {t!("try-again")}
                }
            }
        },
        _ => rsx! {},
    };

    let ready = *listing.state().read() == UseResourceState::Ready;
    let current = current_result(listing.read().as_ref(), &url, ready);
    let content = match current {
        Some(Ok(listing)) => rsx! {
            EntryList { volume: volume.clone(), path: path.clone(), entries: listing.entries }
        },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            div { class: "flex justify-center pt-6",
                ErrorPanel { failure, on_retry: move |()| listing.restart() }
            }
        },
        _ => rsx! {
            p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} }
        },
    };

    rsx! {
        Shell { user, sidebar,
            div { class: "flex flex-col gap-4",
                Breadcrumb { volume: volume.clone(), path: path.clone() }
                {content}
            }
        }
    }
}

/// The fetched result, once the fetch has finished and was for `url`: after navigation the
/// resource still holds the previous folder until the new one arrives.
fn current_result<T: Clone>(fetched: Option<&(String, T)>, url: &str, ready: bool) -> Option<T> {
    fetched
        .filter(|(fetched_url, _)| ready && fetched_url == url)
        .map(|(_, result)| result.clone())
}

#[component]
fn EntryList(volume: Segment, path: Segments, entries: Vec<Entry>) -> Element {
    let locale = i18n().language();

    if entries.is_empty() {
        return rsx! {
            p { class: "py-10 text-center text-sm text-slate-500 dark:text-slate-400", {t!("browse-empty")} }
        };
    }

    rsx! {
        ul { class: "divide-y divide-slate-100 overflow-hidden rounded-lg border border-slate-200 bg-white dark:divide-slate-800 dark:border-slate-800 dark:bg-slate-900",
            for entry in entries {
                li { key: "{entry.name}",
                    match entry.kind {
                        EntryKind::Dir => rsx! {
                            Link {
                                to: Route::Browse { volume: volume.clone(), path: path.child(&entry.name) },
                                class: "{ROW_CLASS} hover:bg-slate-50 dark:hover:bg-slate-800/60",
                                FolderIcon { class: "size-5 shrink-0 text-indigo-500 dark:text-indigo-400" }
                                span { class: "truncate", {entry.name.clone()} }
                            }
                        },
                        EntryKind::File => rsx! {
                            a {
                                href: api::download_url(volume.as_str(), &path.child(&entry.name).joined()),
                                download: "",
                                class: "{ROW_CLASS} hover:bg-slate-50 dark:hover:bg-slate-800/60",
                                FileIcon { class: "size-5 shrink-0 text-slate-400 dark:text-slate-500" }
                                span { class: "truncate", {entry.name.clone()} }
                                if let Some(size) = entry.size {
                                    span { class: "ml-auto shrink-0 text-xs text-slate-500 dark:text-slate-400",
                                        {format_size(size, &locale)}
                                    }
                                }
                            }
                        },
                        EntryKind::Other => rsx! {
                            div { class: "{ROW_CLASS} text-slate-400 dark:text-slate-600",
                                FileIcon { class: "size-5 shrink-0" }
                                span { class: "truncate", {entry.name.clone()} }
                            }
                        },
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
    fn current_result_ignores_a_previous_or_pending_fetch() {
        let fetched = ("/api/volumes/media/entries?path=a".to_owned(), 1);
        let url = fetched.0.clone();
        assert_eq!(current_result(Some(&fetched), &url, true), Some(1));
        assert_eq!(current_result(Some(&fetched), &url, false), None);
        assert_eq!(
            current_result(
                Some(&fetched),
                "/api/volumes/media/entries?path=a%2Fb",
                true
            ),
            None
        );
        assert_eq!(current_result::<i32>(None, &url, true), None);
    }
}
