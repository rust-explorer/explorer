use std::error::Error as StdError;

use openidconnect::core::{CoreAuthenticationFlow, CoreGenderClaim};
use openidconnect::{
    AccessToken, AccessTokenHash, AuthorizationCode, ClaimsVerificationError, CsrfToken, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, Scope, SignatureVerificationError,
    SubjectIdentifier, TokenResponse, UserInfoClaims,
};
use serde_json::Value;
use url::Url;

use super::types::{Client, ExtraClaims};
use super::{Oidc, PendingLogin, SigninError, VerifiedIdentity, error_chain};

const SCOPES: [&str; 3] = ["profile", "email", "groups"];

impl Oidc {
    #[must_use]
    pub fn authorize_url(&self) -> (Url, PendingLogin) {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client()
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
        let client = self.client();
        let tokens = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|err| SigninError::ExchangeFailed(error_chain(&err)))?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(|err| SigninError::ExchangeFailed(error_chain(&err)))?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| SigninError::InvalidToken("token response has no id_token".into()))?;
        let nonce = Nonce::new(pending.nonce);
        let unknown_key = matches!(
            id_token.claims(&client.id_token_verifier(), &nonce),
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey
            ))
        );
        let client = if unknown_key {
            self.refetch_keys(&client).await.unwrap_or(client)
        } else {
            client
        };
        let verifier = client.id_token_verifier();
        let invalid = |err: &dyn StdError| SigninError::InvalidToken(error_chain(err));
        let claims = id_token
            .claims(&verifier, &nonce)
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
                self.userinfo_groups(&client, tokens.access_token(), claims.subject())
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
        client: &Client,
        access_token: &AccessToken,
        subject: &SubjectIdentifier,
    ) -> Result<Option<Vec<String>>, SigninError> {
        let Ok(request) = client
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
    pub fn end_session_url(&self, id_token: &str, post_logout_redirect: &str) -> Option<Url> {
        let mut url = self.end_session_endpoint.clone()?;
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token)
            .append_pair("post_logout_redirect_uri", post_logout_redirect)
            .append_pair("client_id", &self.client_id);
        Some(url)
    }
}

fn parse_groups(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|g| g.as_str().map(str::to_owned))
        .collect()
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
}
