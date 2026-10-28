use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use shared::api::favorites::Favorite;
use shared::api::tasks::Location;

use crate::api::{self, ApiFailure};
use crate::revision::Revision;

/// The user's favorite folders. The list is reloaded after every change and whenever something
/// changed on disk, so a renamed or deleted folder shows as missing.
#[derive(Clone, Copy)]
pub struct FavoritesStore {
    list: Signal<Option<Vec<Favorite>>>,
    failure: Signal<Option<ApiFailure>>,
    load_failed: Signal<bool>,
}

pub fn use_favorites_provider() -> FavoritesStore {
    use_context_provider(|| FavoritesStore {
        list: Signal::new(None),
        failure: Signal::new(None),
        load_failed: Signal::new(false),
    })
}

pub fn use_favorites() -> FavoritesStore {
    use_context()
}

impl FavoritesStore {
    /// `None` until the first load succeeds.
    pub fn list(&self) -> Signal<Option<Vec<Favorite>>> {
        self.list
    }

    /// The last add or remove that failed, until the next one or `clear_failure`.
    pub fn failure(&self) -> Signal<Option<ApiFailure>> {
        self.failure
    }

    /// Whether the last load of the list failed.
    pub fn load_failed(&self) -> bool {
        (self.load_failed)()
    }

    pub fn clear_failure(mut self) {
        if self.failure.peek().is_some() {
            self.failure.set(None);
        }
    }

    pub fn contains(&self, volume: &str, path: &str) -> bool {
        self.list
            .read()
            .as_ref()
            .is_some_and(|list| list.iter().any(|f| f.volume == volume && f.path == path))
    }

    pub fn add(mut self, location: Location) {
        self.failure.set(None);
        spawn_forever(async move {
            let result = api::put_json_body::<_, ()>(api::FAVORITES_URL, &location).await;
            self.finish(result).await;
        });
    }

    pub fn remove(mut self, location: Location) {
        self.failure.set(None);
        spawn_forever(async move {
            let url = api::favorite_url(&location.volume, &location.path);
            let result = api::delete_json::<()>(&url).await;
            self.finish(result).await;
        });
    }

    pub fn reload(self) {
        spawn_forever(async move { self.load().await });
    }

    async fn finish(mut self, result: Result<(), ApiFailure>) {
        if let Err(failure) = result
            && !failure.is_unauthenticated()
        {
            self.failure.set(Some(failure));
        }
        self.load().await;
    }

    async fn load(mut self) {
        let failed = match api::get_json::<Vec<Favorite>>(api::FAVORITES_URL).await {
            Ok(list) => {
                self.list.set(Some(list));
                false
            }
            Err(failure) => !failure.is_unauthenticated(),
        };
        if *self.load_failed.peek() != failed {
            self.load_failed.set(failed);
        }
    }
}

/// Keeps the list of the current page fresh; for pages that show favorites.
pub fn use_favorites_sync() -> FavoritesStore {
    let store = use_favorites();
    let Revision(revision) = use_context();
    use_effect(move || {
        revision();
        store.reload();
    });
    store
}
