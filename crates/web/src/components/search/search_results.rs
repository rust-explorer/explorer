use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::EntryKind;
use shared::api::search::{SearchHit, SearchResults as Results};
use unic_langid::LanguageIdentifier;

use crate::api;
use crate::components::browse::EntryIcon;
use crate::components::common::ErrorPanel;
use crate::format::{format_modified, format_size, now_ms};
use crate::revision::Revision;
use crate::routes::{Route, Segments};

const MUTED: &str = "text-slate-500 dark:text-slate-400";
const LINK: &str = "truncate rounded focus-visible:outline-2 focus-visible:outline-indigo-600";

/// The hits for `query` in the whole volume, in place of the folder's listing. A new query or
/// leaving the page aborts the request in flight.
#[component]
pub fn SearchResults(volume: String, query: String) -> Element {
    let Revision(revision) = use_context();
    let url = api::search_url(&volume, &query);
    let mut results = use_resource(use_reactive!(|url| async move {
        revision();
        let guard = api::AbortGuard::new();
        let result = api::get_json_abortable::<Results>(&url, &guard.signal()).await;
        (url, result)
    }));
    let ready = *results.state().read() == UseResourceState::Ready;
    let current = results
        .read()
        .clone()
        .filter(|(fetched, _)| *fetched == url && ready)
        .map(|(_, result)| result);

    match current {
        Some(Ok(found)) => rsx! { Hits { found } },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            div { class: "flex justify-center pt-6",
                ErrorPanel { failure, on_retry: move |()| results.restart() }
            }
        },
        _ => rsx! {
            p { role: "status", class: "text-sm {MUTED}", {t!("search-searching")} }
        },
    }
}

#[component]
fn Hits(found: Results) -> Element {
    let locale = i18n().language();
    let now = now_ms();
    let count = found.hits.len();

    rsx! {
        h2 { class: "mb-2 text-sm font-semibold text-gray-900 dark:text-white",
            if count == 0 {
                {t!("search-empty", query: found.query.clone(), volume: found.volume.clone())}
            } else {
                {t!("search-results", count: count, query: found.query.clone(), volume: found.volume.clone())}
            }
        }
        if found.truncated {
            p { class: "mb-2 text-sm {MUTED}", {t!("search-truncated", count: count)} }
        }
        if count > 0 {
            ul { class: "overflow-hidden rounded-lg border border-slate-200 bg-white text-sm dark:border-slate-800 dark:bg-slate-900",
                for hit in found.hits.iter().cloned() {
                    li {
                        key: "{hit.path}",
                        class: "flex min-h-11 items-center gap-3 border-b border-slate-100 px-3 py-1.5 last:border-b-0 hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-800/60",
                        HitRow { volume: found.volume.clone(), hit, now, locale: locale.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn HitRow(volume: String, hit: SearchHit, now: i64, locale: LanguageIdentifier) -> Element {
    let (parent, name) = split_path(&hit.path);
    let size = hit
        .size
        .filter(|_| hit.kind == EntryKind::File)
        .map(|size| format_size(size, &locale));
    let modified = hit.modified_ms.map(|ms| format_modified(ms, now, &locale));
    let details = [size.clone(), modified.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let parent_label = if parent.is_empty() { &volume } else { parent };

    rsx! {
        EntryIcon { kind: hit.kind, name: name.to_owned() }
        div { class: "flex min-w-0 flex-1 flex-col",
            if hit.kind == EntryKind::Dir {
                Link {
                    to: Route::Browse { volume: volume.clone().into(), path: segments(&hit.path) },
                    class: LINK,
                    {name.to_owned()}
                }
            } else {
                a {
                    href: api::download_url(&volume, &hit.path),
                    download: "",
                    class: LINK,
                    {name.to_owned()}
                }
            }
            div { class: "flex min-w-0 items-center gap-1.5 text-xs {MUTED}",
                Link {
                    to: Route::Browse { volume: volume.clone().into(), path: segments(parent) },
                    class: "{LINK} hover:text-indigo-600 hover:underline dark:hover:text-indigo-400",
                    {parent_label.to_owned()}
                }
                if !details.is_empty() {
                    span { class: "shrink-0 md:hidden", "· {details}" }
                }
            }
        }
        span { class: "hidden w-24 shrink-0 truncate md:block {MUTED}", {size.unwrap_or_default()} }
        span { class: "hidden w-36 shrink-0 truncate md:block {MUTED}", {modified.unwrap_or_default()} }
    }
}

/// The folder part and the name of a volume-relative path.
fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn segments(path: &str) -> Segments {
    path.split('/')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_into_folder_and_name() {
        assert_eq!(split_path("a/b/c.txt"), ("a/b", "c.txt"));
        assert_eq!(split_path("c.txt"), ("", "c.txt"));
        assert_eq!(segments("a/b").joined(), "a/b");
        assert_eq!(segments("").joined(), "");
    }
}
