use std::error::Error as StdError;
use std::fmt;
use std::fmt::Write as _;
use std::time::Duration;

use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
    CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreRevocableToken, CoreRevocationErrorResponse, CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AccessToken, AccessTokenHash, AdditionalClaims, AuthorizationCode, ClientId, ClientSecret,
    CsrfToken, DiscoveryError, EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IdTokenFields, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, ProviderMetadataWithLogout, RedirectUrl, Scope, StandardErrorResponse,
    StandardTokenResponse, SubjectIdentifier, TokenResponse, UserInfoClaims, reqwest,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

use crate::config::OidcConfig;

const SCOPES: [&str; 3] = ["profile", "email", "groups"];
const CALLBACK_PATH: &str = "auth/callback";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Every claim of the ID token or userinfo response not covered by the standard set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ExtraClaims {
    #[serde(flatten)]
    claims: Map<String, Value>,
}

impl AdditionalClaims for ExtraClaims {}

type Tokens = StandardTokenResponse<
    IdTokenFields<
        ExtraClaims,
        EmptyExtraTokenFields,
        CoreGenderClaim,
        CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm,
    >,
    CoreTokenType,
>;

type Client = openidconnect::Client<
    ExtraClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    Tokens,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct Oidc {
    client: Client,
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

impl Oidc {
    pub async fn discover(cfg: &OidcConfig, public_url: &Url) -> Result<Self, OidcError> {
        Self::discover_using(cfg, public_url, &http_client()?).await
    }

    /// Retries transient failures (provider or its proxy down or answering garbage), so Explorer
    /// survives the provider starting at the same time. Configuration errors fail at once.
    pub async fn discover_with_retry(
        cfg: &OidcConfig,
        public_url: &Url,
        attempts: u32,
        delay: Duration,
    ) -> Result<Self, OidcError> {
        let http = http_client()?;
        let attempts = attempts.max(1);
        let mut attempt = 1;
        loop {
            match Self::discover_using(cfg, public_url, &http).await {
                Ok(oidc) => return Ok(oidc),
                Err(err) if err.is_transient() => {
                    tracing::warn!("OIDC discovery failed (attempt {attempt}/{attempts}): {err}");
                    if attempt == attempts {
                        let OidcError::Discovery { issuer, reason, .. } = err else {
                            unreachable!("only discovery errors are transient");
                        };
                        return Err(OidcError::Unreachable {
                            issuer,
                            attempts,
                            reason,
                        });
                    }
                }
                Err(err) => return Err(err),
            }
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }

    async fn discover_using(
        cfg: &OidcConfig,
        public_url: &Url,
        http: &reqwest::Client,
    ) -> Result<Self, OidcError> {
        let issuer_url =
            IssuerUrl::new(cfg.issuer.clone()).map_err(|err| OidcError::InvalidIssuer {
                issuer: cfg.issuer.clone(),
                reason: err.to_string(),
            })?;
        let metadata = ProviderMetadataWithLogout::discover_async(issuer_url, http)
            .await
            .map_err(|err| OidcError::Discovery {
                issuer: cfg.issuer.clone(),
                transient: is_transient(&err),
                reason: error_chain(&err),
            })?;
        let end_session_endpoint = metadata
            .additional_metadata()
            .end_session_endpoint
            .as_ref()
            .map(|url| url.url().clone());
        let redirect = public_url
            .join(CALLBACK_PATH)
            .expect("public_url is an absolute base URL");
        let client = Client::from_provider_metadata(
            metadata,
            ClientId::new(cfg.client_id.clone()),
            Some(ClientSecret::new(
                cfg.client_secret.expose_secret().to_owned(),
            )),
        )
        .set_redirect_uri(RedirectUrl::from_url(redirect));
        Ok(Self {
            client,
            http: http.clone(),
            end_session_endpoint,
            client_id: cfg.client_id.clone(),
            groups_claim: cfg.groups_claim.clone(),
        })
    }

    #[must_use]
    pub fn authorize_url(&self) -> (Url, PendingLogin) {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scopes(SCOPES.map(|s| Scope::new(s.to_owned())))
            .set_pkce_challenge(challenge)
            .url();
        let pending = PendingLogin {
            csrf_state: state.into_secret(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.into_secret(),
            return_to: "/".to_owned(),
        };
        (url, pending)
    }

    pub async fn exchange(
        &self,
        code: String,
        pending: PendingLogin,
    ) -> Result<VerifiedIdentity, SigninError> {
        let tokens = self
            .client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|err| SigninError::ExchangeFailed(error_chain(&err)))?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(|err| SigninError::ExchangeFailed(error_chain(&err)))?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| SigninError::InvalidToken("token response has no id_token".into()))?;
        let verifier = self.client.id_token_verifier();
        let invalid = |err: &dyn StdError| SigninError::InvalidToken(error_chain(err));
        let claims = id_token
            .claims(&verifier, &Nonce::new(pending.nonce))
            .map_err(|e| invalid(&e))?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                tokens.access_token(),
                id_token.signing_alg().map_err(|e| invalid(&e))?,
                id_token.signing_key(&verifier).map_err(|e| invalid(&e))?,
            )
            .map_err(|e| invalid(&e))?;
            if actual != *expected {
                return Err(SigninError::InvalidToken(
                    "access token hash mismatch".into(),
                ));
            }
        }

        let groups = match claims.additional_claims().claims.get(&self.groups_claim) {
            Some(value) => parse_groups(value),
            None => {
                self.userinfo_groups(tokens.access_token(), claims.subject())
                    .await?
            }
        }
        .ok_or(SigninError::MissingGroupsClaim)?;

        let sub = claims.subject().to_string();
        let name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| claims.preferred_username().map(|u| u.to_string()))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| sub.clone());
        Ok(VerifiedIdentity {
            sub,
            name,
            email: claims.email().map(|e| e.to_string()),
            groups,
            id_token: id_token.to_string(),
        })
    }

    async fn userinfo_groups(
        &self,
        access_token: &AccessToken,
        subject: &SubjectIdentifier,
    ) -> Result<Option<Vec<String>>, SigninError> {
        let Ok(request) = self
            .client
            .user_info(access_token.clone(), Some(subject.clone()))
        else {
            return Ok(None);
        };
        let info: UserInfoClaims<ExtraClaims, CoreGenderClaim> = request
            .request_async(&self.http)
            .await
            .map_err(|err| SigninError::ExchangeFailed(error_chain(&err)))?;
        Ok(info
            .additional_claims()
            .claims
            .get(&self.groups_claim)
            .and_then(parse_groups))
    }

    #[must_use]
    pub fn end_session_url(&self, id_token: &str, post_logout_redirect: &Url) -> Option<Url> {
        let mut url = self.end_session_endpoint.clone()?;
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token)
            .append_pair("post_logout_redirect_uri", post_logout_redirect.as_str())
            .append_pair("client_id", &self.client_id);
        Some(url)
    }
}

fn http_client() -> Result<reqwest::Client, OidcError> {
    reqwest::Client::builder()
        // Following redirects would let a provider response steer requests elsewhere (SSRF).
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|err| OidcError::HttpClient(error_chain(&err)))
}

/// A reverse proxy in front of a restarting provider can answer anything (404, 502, an HTML
/// maintenance page), so only errors that come from the configuration itself are final.
fn is_transient<E: StdError>(err: &DiscoveryError<E>) -> bool {
    !matches!(
        err,
        DiscoveryError::Validation(_) | DiscoveryError::UrlParse(_) | DiscoveryError::Other(_)
    )
}

fn parse_groups(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|g| g.as_str().map(str::to_owned))
        .collect()
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
    use serde_json::json;

    use super::*;

    #[test]
    fn groups_must_be_an_array_of_strings() {
        assert_eq!(
            parse_groups(&json!(["a", "b"])),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(parse_groups(&json!([])), Some(vec![]));
        assert_eq!(parse_groups(&json!("a")), None);
        assert_eq!(parse_groups(&json!(["a", 1])), None);
        assert_eq!(parse_groups(&Value::Null), None);
    }

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

    #[test]
    fn everything_but_configuration_errors_is_transient() {
        type E = DiscoveryError<std::io::Error>;
        let response = |code| {
            E::Response(
                openidconnect::http::StatusCode::from_u16(code).unwrap(),
                vec![],
                String::new(),
            )
        };
        assert!(is_transient(&E::Request(std::io::Error::other("down"))));
        assert!(is_transient(&response(503)));
        assert!(is_transient(&response(404)));
        assert!(is_transient(&response(200)));
        assert!(!is_transient(&E::Validation("issuer".into())));
        assert!(!is_transient(&E::UrlParse(url::ParseError::EmptyHost)));
        assert!(!is_transient(&E::Other("prepare request".into())));
    }
}
