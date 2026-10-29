use std::error::Error as StdError;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use openidconnect::{
    ClientId, ClientSecret, DiscoveryError, IssuerUrl, ProviderMetadataWithLogout, RedirectUrl,
    reqwest,
};
use secrecy::ExposeSecret;
use url::Url;

use super::types::Client;
use super::{Oidc, OidcError, error_chain};
use crate::config::OidcConfig;

const CALLBACK_PATH: &str = "auth/callback";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

impl Oidc {
    pub async fn discover(cfg: &OidcConfig, public_url: &Url) -> Result<Self, OidcError> {
        Self::discover_using(cfg, public_url, &http_client(&cfg.ca_certs)?).await
    }

    /// Retries transient failures (provider or its proxy down or answering garbage), so Explorer
    /// survives the provider starting at the same time. Configuration errors fail at once.
    pub async fn discover_with_retry(
        cfg: &OidcConfig,
        public_url: &Url,
        attempts: u32,
        delay: Duration,
    ) -> Result<Self, OidcError> {
        let http = http_client(&cfg.ca_certs)?;
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
        let client_secret = ClientSecret::new(cfg.client_secret.expose_secret().to_owned());
        let redirect = RedirectUrl::from_url(redirect);
        let client = build_client(&metadata, &cfg.client_id, &client_secret, &redirect);
        Ok(Self {
            client: RwLock::new(Arc::new(client)),
            metadata,
            client_secret,
            redirect,
            keys_fetched_at: tokio::sync::Mutex::new(None),
            http: http.clone(),
            end_session_endpoint,
            client_id: cfg.client_id.clone(),
            groups_claim: cfg.groups_claim.clone(),
        })
    }
}

pub(super) fn build_client(
    metadata: &ProviderMetadataWithLogout,
    client_id: &str,
    client_secret: &ClientSecret,
    redirect: &RedirectUrl,
) -> Client {
    Client::from_provider_metadata(
        metadata.clone(),
        ClientId::new(client_id.to_owned()),
        Some(client_secret.clone()),
    )
    .set_redirect_uri(redirect.clone())
}

/// `ca_certs` are trusted on top of the built-in roots, never instead of them.
fn http_client(ca_certs: &[reqwest::Certificate]) -> Result<reqwest::Client, OidcError> {
    let mut builder = reqwest::Client::builder();
    for cert in ca_certs {
        builder = builder.add_root_certificate(cert.clone());
    }
    builder
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TEST_CA_PEM;

    #[test]
    fn client_builds_with_and_without_extra_roots() {
        assert!(http_client(&[]).is_ok());
        let certs = reqwest::Certificate::from_pem_bundle(TEST_CA_PEM.as_bytes()).unwrap();
        assert!(http_client(&certs).is_ok());
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
