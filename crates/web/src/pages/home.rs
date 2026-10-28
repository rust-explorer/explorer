use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::{Me, VolumeInfo};

use crate::api;
use crate::components::{ErrorPanel, Notice, Shell};
use crate::routes::{Route, Segments};

/// `/` opens the first volume, or explains that there is none.
#[component]
pub fn Home() -> Element {
    let navigator = navigator();
    let me = use_resource(|| api::get_json::<Me>("/api/me"));
    let mut volumes = use_resource(move || async move {
        let volumes = api::get_json::<Vec<VolumeInfo>>("/api/volumes").await;
        if let Ok(Some(first)) = volumes.as_ref().map(|volumes| volumes.first()) {
            navigator.replace(Route::Browse {
                volume: first.name.clone().into(),
                path: Segments::default(),
            });
        }
        volumes
    });

    let user = me.read().clone().and_then(Result::ok);
    let result = volumes.read().clone();
    match result {
        Some(Ok(list)) if list.is_empty() => rsx! {
            Shell { user,
                Notice { title: t!("no-volumes-title"), message: t!("no-volumes-message") }
            }
        },
        Some(Err(failure)) if !failure.is_unauthenticated() => rsx! {
            Shell { user,
                ErrorPanel { failure, on_retry: move |()| volumes.restart() }
            }
        },
        // Loading, redirecting to the first volume, or to the sign-in page.
        _ => rsx! {
            Shell { user,
                p { class: "text-sm text-slate-500 dark:text-slate-400", {t!("loading")} }
            }
        },
    }
}
