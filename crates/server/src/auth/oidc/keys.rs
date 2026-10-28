use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use openidconnect::JsonWebKeySet;

use super::discovery::build_client;
use super::types::Client;
use super::{Oidc, error_chain};

/// A provider rotating its keys needs one fetch; a token signed with a key nobody publishes must
/// not make every sign-in hit the provider.
const REFETCH_INTERVAL: Duration = Duration::from_secs(60);

impl Oidc {
    pub(super) fn client(&self) -> Arc<Client> {
        Arc::clone(&self.client.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Fetches the provider's signing keys again after an ID token signed with a key that `stale`
    /// does not know, at most once a minute. Returns the client to verify with again, or `None`
    /// when there is nothing new to try.
    pub(super) async fn refetch_keys(&self, stale: &Arc<Client>) -> Option<Arc<Client>> {
        let mut fetched_at = self.keys_fetched_at.lock().await;
        let current = self.client();
        if !Arc::ptr_eq(&current, stale) {
            return Some(current);
        }
        if fetched_at.is_some_and(|at| at.elapsed() < REFETCH_INTERVAL) {
            return None;
        }
        *fetched_at = Some(Instant::now());
        let jwks = match JsonWebKeySet::fetch_async(self.metadata.jwks_uri(), &self.http).await {
            Ok(jwks) => jwks,
            Err(err) => {
                tracing::warn!(
                    error = error_chain(&err),
                    "cannot fetch the provider's keys"
                );
                return None;
            }
        };
        let metadata = self.metadata.clone().set_jwks(jwks);
        let client = Arc::new(build_client(
            &metadata,
            &self.client_id,
            &self.client_secret,
            &self.redirect,
        ));
        *self.client.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&client);
        tracing::info!("provider signing keys fetched again");
        Some(client)
    }
}
