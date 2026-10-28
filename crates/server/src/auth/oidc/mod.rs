//! OIDC sign-in on top of the `openidconnect` crate: provider discovery, the authorization code
//! flow with PKCE, ID token verification and the end-session URL.

mod discovery;
mod flow;
mod keys;
mod types;

use std::error::Error as StdError;
use std::fmt;
use std::fmt::Write as _;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use openidconnect::{ClientSecret, ProviderMetadataWithLogout, RedirectUrl, reqwest};
use serde::{Deserialize, Serialize};
use url::Url;

use types::Client;

pub struct Oidc {
    /// Replaced when the provider's signing keys are fetched again.
    client: RwLock<Arc<Client>>,
    metadata: ProviderMetadataWithLogout,
    client_secret: ClientSecret,
    redirect: RedirectUrl,
    keys_fetched_at: tokio::sync::Mutex<Option<Instant>>,
    http: reqwest::Client,
    end_session_endpoint: Option<Url>,
    client_id: String,
    groups_claim: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("invalid OIDC issuer URL `{issuer}`: {reason}")]
    InvalidIssuer { issuer: String, reason: String },
    #[error("cannot build the OIDC HTTP client: {0}")]
    HttpClient(String),
    #[error("OIDC discovery for issuer `{issuer}` failed: {reason}")]
    Discovery {
        issuer: String,
        reason: String,
        transient: bool,
    },
    #[error("OIDC issuer `{issuer}` still unreachable after {attempts} attempts: {reason}")]
    Unreachable {
        issuer: String,
        attempts: u32,
        reason: String,
    },
}

impl OidcError {
    fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Discovery {
                transient: true,
                ..
            }
        )
    }
}

/// Secret per-login values; deliberately not `Debug` so they cannot end up in logs.
#[derive(Clone, Serialize, Deserialize)]
pub struct PendingLogin {
    pub csrf_state: String,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: String,
}

#[derive(Clone)]
pub struct VerifiedIdentity {
    pub sub: String,
    pub name: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
    pub id_token: String,
}

impl fmt::Debug for VerifiedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifiedIdentity")
            .field("sub", &self.sub)
            .field("name", &self.name)
            .field("email", &self.email)
            .field("groups", &self.groups)
            .field("id_token", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SigninError {
    #[error("code exchange failed: {0}")]
    ExchangeFailed(String),
    #[error("invalid ID token: {0}")]
    InvalidToken(String),
    #[error("groups claim missing or not an array of strings")]
    MissingGroupsClaim,
}

impl SigninError {
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            Self::ExchangeFailed(_) => "exchange_failed",
            Self::InvalidToken(_) => "invalid_token",
            Self::MissingGroupsClaim => "missing_groups_claim",
        }
    }
}

/// Joins an error with its sources: openidconnect's top-level messages omit the cause.
fn error_chain(err: &dyn StdError) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !message.ends_with(&cause_text) {
            let _ = write!(message, ": {cause_text}");
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signin_error_reasons() {
        assert_eq!(
            SigninError::ExchangeFailed(String::new()).reason(),
            "exchange_failed"
        );
        assert_eq!(
            SigninError::InvalidToken(String::new()).reason(),
            "invalid_token"
        );
        assert_eq!(
            SigninError::MissingGroupsClaim.reason(),
            "missing_groups_claim"
        );
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] std::io::Error);

    #[test]
    fn error_chain_includes_sources() {
        let err = Outer(std::io::Error::other("connection refused"));
        assert_eq!(error_chain(&err), "outer: connection refused");
    }
}
