use std::time::Instant;

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use shared::api::browse::EntryKind;
use shared::api::search::{MIN_QUERY_CHARS, SearchHit, SearchResults};

use tokio::sync::{Semaphore, SemaphorePermit};

use super::{SEARCH_CONCURRENCY, SEARCH_TIMEOUT};
use crate::auth::{Action, CurrentUser};
use crate::browse::epoch_millis;
use crate::http::AppState;
use crate::http::access::{CancelOnDrop, authorize, blocking_volume, invalid_query};
use crate::http::error::ApiError;
use crate::storage::{self, SEARCH_LIMIT};

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
}

static SEARCHES: Semaphore = Semaphore::const_new(SEARCH_CONCURRENCY);

/// Waits for one of the [`SEARCH_CONCURRENCY`] search slots.
async fn slot() -> Result<SemaphorePermit<'static>, ApiError> {
    SEARCHES.acquire().await.map_err(ApiError::internal)
}

pub async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    volume: Result<Path<String>, PathRejection>,
    query: Result<Query<SearchQuery>, QueryRejection>,
) -> Result<Json<SearchResults>, ApiError> {
    let (Ok(Path(volume)), Ok(Query(query))) = (volume, query) else {
        return Err(invalid_query());
    };
    authorize(&state, &user, &volume, "", Action::Read)?;
    let needle = query.q.trim().to_owned();
    if needle.chars().count() < MIN_QUERY_CHARS {
        return Err(invalid_query());
    }

    let (_guard, cancel) = CancelOnDrop::new();
    let slot = slot().await?;
    let started = Instant::now();
    let outcome = blocking_volume(&state.volumes, &volume, {
        let needle = needle.clone();
        move |vol| {
            let _slot = slot;
            vol.search(
                &needle,
                SEARCH_LIMIT,
                Instant::now() + SEARCH_TIMEOUT,
                &cancel,
            )
        }
    })
    .await?
    .map_err(|err| ApiError::from_storage(&volume, err))?;
    tracing::debug!(
        user = user.sub,
        volume,
        query = needle,
        hits = outcome.hits.len(),
        truncated = outcome.truncated,
        elapsed_ms = started.elapsed().as_millis(),
        "search"
    );

    let hits = outcome
        .hits
        .into_iter()
        .map(|hit| SearchHit {
            path: hit.path.as_str().to_owned(),
            kind: match hit.kind {
                storage::EntryKind::Dir => EntryKind::Dir,
                storage::EntryKind::File | storage::EntryKind::Other => EntryKind::File,
            },
            size: hit.size,
            modified_ms: hit.modified.and_then(epoch_millis),
        })
        .collect();
    Ok(Json(SearchResults {
        volume,
        query: needle,
        hits,
        truncated: outcome.truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn searches_wait_for_a_free_slot() {
        let held = hold_all().await;
        let wait = std::time::Duration::from_millis(50);
        assert!(tokio::time::timeout(wait, slot()).await.is_err());
        drop(held);
        assert!(tokio::time::timeout(wait, slot()).await.is_ok());
    }

    async fn hold_all() -> Vec<SemaphorePermit<'static>> {
        let mut held = Vec::new();
        for _ in 0..SEARCH_CONCURRENCY {
            held.push(slot().await.unwrap());
        }
        held
    }
}
