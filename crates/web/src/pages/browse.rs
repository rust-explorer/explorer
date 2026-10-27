use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::auth::Me;
use shared::api::browse::{Listing, VolumeInfo};

use crate::api;
use crate::components::{
    Breadcrumb, DropZone, ErrorPanel, FileTable, NewFolderButton, RefreshButton, SearchField,
    SearchResults, Shell, Sidebar, UploadButton,
};
use crate::favorites::use_favorites_sync;
use crate::revision::Revision;
use crate::routes::{Segment, Segments};
use crate::search::{SearchQuery, effective_query};
use crate::selection::Selection;

#[component]
pub fn Browse(volume: Segment, path: Segments) -> Element {
    let Revision(revision) = use_context();
    use_favorites_sync();
    let mut me = use_resource(|| api::get_json::<Me>("/api/me"));
    let mut volumes = use_resource(move || {
        revision();
        api::get_json::<Vec<VolumeInfo>>("/api/volumes")
    });
    let url = api::entries_url(volume.as_str(), &path.joined());
    let mut listing = use_resource(use_reactive!(|url| async move {
        revision();
        let result = api::get_json::<Listing>(&url)
            .await
            .map(|listing| Rc::new(listing.entries));
        (url, result)
    }));

    let mut selection = use_signal(Selection::default);
    let SearchQuery(mut query) = use_context();
    use_effect(use_reactive!(|url| {
        let _ = url;
        selection.write().clear();
        query.set(String::new());
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

    let read_only = volumes
        .read()
        .as_ref()
        .and_then(|volumes| volumes.as_ref().ok())
        .and_then(|volumes| volumes.iter().find(|info| info.name == volume.as_str()))
        .map(|info| info.read_only);
    let busy = !matches!(*listing.state().read(), UseResourceState::Ready);
    let actions = read_only.map(|read_only| {
        rsx! {
            RefreshButton { busy }
            NewFolderButton { volume: volume.as_str().to_owned(), path: path.joined(), read_only }
            UploadButton { volume: volume.as_str().to_owned(), path: path.joined(), read_only }
        }
    });

    let search = rsx! {
        SearchField { key: "{url}", volume: volume.as_str().to_owned() }
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
    let searching = effective_query(&query.read()).map(ToOwned::to_owned);
    let content = match current {
        _ if searching.is_some() => rsx! {
            SearchResults {
                volume: volume.as_str().to_owned(),
                query: searching.clone().unwrap_or_default(),
            }
        },
        Some(Ok(entries)) => rsx! {
            FileTable {
                volume: volume.clone(),
                path: path.clone(),
                entries,
                read_only: read_only.unwrap_or(true),
                selection,
            }
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
        Shell { user, sidebar, actions, search,
            DropZone { volume: volume.as_str().to_owned(), path: path.joined(), read_only,
                Breadcrumb { volume: volume.clone(), path: path.clone() }
                {content}
            }
        }
    }
}

/// The fetched result when it was for `url`: after navigation the resource still holds the
/// previous folder until the new one arrives. A refetch of the same folder keeps showing it.
fn current_result<T: Clone, E: Clone>(
    fetched: Option<&(String, Result<T, E>)>,
    url: &str,
    ready: bool,
) -> Option<Result<T, E>> {
    fetched
        .filter(|(fetched_url, result)| fetched_url == url && (ready || result.is_ok()))
        .map(|(_, result)| result.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "/api/volumes/media/entries?path=a";
    const OTHER: &str = "/api/volumes/media/entries?path=a%2Fb";

    fn fetched(result: Result<i32, &'static str>) -> (String, Result<i32, &'static str>) {
        (URL.to_owned(), result)
    }

    #[test]
    fn current_result_ignores_a_previous_or_pending_fetch() {
        let ok = fetched(Ok(1));
        assert_eq!(current_result(Some(&ok), URL, true), Some(Ok(1)));
        assert_eq!(current_result(Some(&ok), OTHER, true), None);
        assert_eq!(current_result(Some(&ok), OTHER, false), None);
        assert_eq!(current_result::<i32, &str>(None, URL, true), None);
    }

    #[test]
    fn current_result_keeps_stale_listing_while_refreshing() {
        assert_eq!(
            current_result(Some(&fetched(Ok(1))), URL, false),
            Some(Ok(1))
        );
        assert_eq!(
            current_result(Some(&fetched(Err("gone"))), URL, false),
            None
        );
        assert_eq!(current_result(Some(&fetched(Ok(1))), OTHER, false), None);
    }
}
