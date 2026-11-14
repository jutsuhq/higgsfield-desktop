use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use super::client::OAuthClient;
use super::metadata::GrantType;
use super::pkce::{Pkce, PkceHasher, Sha256Hasher, random_url_safe_string};
use super::token_set::TokenSet;
use crate::core::HiggsfieldError;
use crate::mcp::error::{McpError, OAuthError};
use crate::mcp::http::{is_same_resource, query_items, query_value};

/// Builds the authorization URL for a redirect URI.
pub type MakeUrl<'a> = &'a (dyn Fn(&str) -> Result<Url, McpError> + Send + Sync);

/// The browser step of the authorization code flow: decides where the redirect arrives, opens the authorization
/// URL built for that redirect URI and returns the redirect.
#[async_trait]
pub trait AuthorizationPresenter: Send + Sync {
    async fn authorize(&self, make_url: MakeUrl<'_>) -> Result<AuthorizationRedirect, McpError>;
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AuthorizationRedirect {
    /// The full redirect, with `code` or `error` and `state` in its query.
    pub callback: Url,
    /// The redirect URI the authorization URL was built with; the token request has to repeat it.
    pub redirect_uri: String,
}

impl std::fmt::Debug for AuthorizationRedirect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizationRedirect")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// Authorization code flow with PKCE S256, `state` and `resource` (OAuth 2.1, RFC 7636, RFC 8707).
#[derive(Clone)]
pub struct PkceAuthorizer {
    client: OAuthClient,
    presenter: Arc<dyn AuthorizationPresenter>,
    hasher: Arc<dyn PkceHasher>,
}

impl std::fmt::Debug for PkceAuthorizer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PkceAuthorizer")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl PkceAuthorizer {
    pub fn new(client: OAuthClient, presenter: Arc<dyn AuthorizationPresenter>) -> Self {
        Self::with_hasher(client, presenter, Arc::new(Sha256Hasher))
    }

    pub fn with_hasher(
        client: OAuthClient,
        presenter: Arc<dyn AuthorizationPresenter>,
        hasher: Arc<dyn PkceHasher>,
    ) -> Self {
        Self {
            client,
            presenter,
            hasher,
        }
    }

    pub async fn authorize(&self) -> Result<TokenSet, McpError> {
        let server = &self.client.server;
        let endpoint = server
            .authorization_endpoint
            .as_ref()
            .filter(|_| server.supports_authorization_code())
            .ok_or(OAuthError::UnsupportedFlow)?;
        let pkce = Pkce::new(self.hasher.as_ref()).map_err(|_| HiggsfieldError::NotSent)?;
        let state = random_url_safe_string(32).map_err(|_| HiggsfieldError::NotSent)?;
        let make_url = |redirect_uri: &str| Ok(self.authorization_url(endpoint, &pkce, &state, redirect_uri));
        let redirect = self.presenter.authorize(&make_url).await?;
        let code = Self::code(
            &redirect.callback,
            &state,
            &server.issuer,
            server.authorization_response_iss_parameter_supported,
        )?;
        self.client
            .request_tokens(BTreeMap::from([
                ("grant_type".to_owned(), GrantType::AUTHORIZATION_CODE.to_owned()),
                ("code".to_owned(), code),
                ("redirect_uri".to_owned(), redirect.redirect_uri),
                ("code_verifier".to_owned(), pkce.verifier.clone()),
                ("resource".to_owned(), self.client.resource.as_str().to_owned()),
            ]))
            .await
    }

    /// `endpoint` with its own query, then the authorization request's parameters.
    pub(crate) fn authorization_url(
        &self,
        endpoint: &Url,
        pkce: &Pkce,
        state: &str,
        redirect_uri: &str,
    ) -> Url {
        let parameters = [
            ("response_type", "code"),
            ("client_id", &self.client.client_id),
            ("redirect_uri", redirect_uri),
            ("scope", &self.client.scope),
            ("state", state),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("resource", self.client.resource.as_str()),
        ];
        let added = parameters
            .iter()
            .map(|(name, value)| format!("{name}={}", query_escaped(value)))
            .collect::<Vec<_>>()
            .join("&");
        let mut url = endpoint.clone();
        let query = match endpoint.query() {
            Some(existing) if !existing.is_empty() => format!("{existing}&{added}"),
            _ => added,
        };
        url.set_query(Some(&query));
        url
    }

    /// The `code` of a redirect, after checking `state` first so a forged redirect never reaches the token
    /// endpoint. RFC 9207: `iss` must name `issuer`. It may be missing only when the server doesn't advertise
    /// sending it (`requires_issuer` false).
    pub(crate) fn code(
        callback: &Url,
        expected_state: &str,
        issuer: &Url,
        requires_issuer: bool,
    ) -> Result<String, OAuthError> {
        let items = query_items(callback);
        let value = |name: &str| {
            items
                .iter()
                .find(|(item, _)| item == name)
                .and_then(|(_, value)| value.clone())
        };
        if value("state").as_deref() != Some(expected_state) {
            return Err(OAuthError::StateMismatch);
        }
        if let Some(iss) = value("iss") {
            if !Url::parse(&iss).is_ok_and(|iss| is_same_resource(&iss, issuer)) {
                return Err(OAuthError::IssuerMismatch);
            }
        } else if requires_issuer {
            return Err(OAuthError::IssuerMismatch);
        }
        if let Some(error) = value("error") {
            return Err(if error == "access_denied" {
                OAuthError::AccessDenied
            } else {
                OAuthError::Server {
                    code: error,
                    description: value("error_description"),
                }
            });
        }
        value("code")
            .filter(|code| !code.is_empty())
            .ok_or(OAuthError::InvalidCallback)
    }
}

/// A query value as `URLComponents` writes it: what a query may hold stays, the rest is percent-encoded, and
/// `&`, `=` and `+` are escaped so the value reads back as it was.
fn query_escaped(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$'()*,;:@/?".contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            let _ = write!(escaped, "%{byte:02X}");
        }
    }
    escaped
}

/// The `state` of an authorization URL, which the redirect has to carry back.
pub(crate) fn state_of(url: &Url) -> Option<String> {
    query_value(url, "state")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLERK: &str = "https://clerk.higgsfield.ai";
    const CLERK_ISS: &str = "iss=https%3A%2F%2Fclerk.higgsfield.ai";

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn callback_with_foreign_state_is_rejected() {
        let callback = url(&format!(
            "http://127.0.0.1:47613/callback?code=abc&state=other&{CLERK_ISS}"
        ));
        assert_eq!(
            PkceAuthorizer::code(&callback, "mine", &url(CLERK), true),
            Err(OAuthError::StateMismatch)
        );
        let denied = url(&format!(
            "http://127.0.0.1:47613/callback?error=access_denied&state=mine&{CLERK_ISS}"
        ));
        assert_eq!(
            PkceAuthorizer::code(&denied, "mine", &url(CLERK), true),
            Err(OAuthError::AccessDenied)
        );
    }

    /// The code of `http://127.0.0.1:47613/callback?<query>`, for state `s` at clerk.
    fn code(query: &str, requires_issuer: bool) -> Result<String, OAuthError> {
        let callback = url(&format!("http://127.0.0.1:47613/callback?{query}"));
        PkceAuthorizer::code(&callback, "s", &url(CLERK), requires_issuer)
    }

    #[test]
    fn callback_issuer_is_checked_when_present() {
        assert_eq!(
            code(&format!("code=abc&state=s&{CLERK_ISS}"), false).as_deref(),
            Ok("abc")
        );
        assert_eq!(code("code=abc&state=s", false).as_deref(), Ok("abc"));
        assert_eq!(
            code("code=abc&state=s&iss=https%3A%2F%2Fevil.example", false),
            Err(OAuthError::IssuerMismatch)
        );
    }

    #[test]
    fn callback_issuer_is_required_when_the_server_advertises_it() {
        assert_eq!(
            code(&format!("code=abc&state=s&{CLERK_ISS}"), true).as_deref(),
            Ok("abc")
        );
        assert_eq!(code("code=abc&state=s", true), Err(OAuthError::IssuerMismatch));
        assert_eq!(
            code("error=access_denied&state=s", true),
            Err(OAuthError::IssuerMismatch)
        );
    }

    #[test]
    fn callback_issuer_may_differ_by_a_trailing_slash() {
        let slash = "code=abc&state=s&iss=https%3A%2F%2Fclerk.higgsfield.ai%2F";
        assert_eq!(code(slash, true).as_deref(), Ok("abc"));
        let other = "code=abc&state=s&iss=https%3A%2F%2Fclerk.higgsfield.ai%2Fother";
        assert_eq!(code(other, true), Err(OAuthError::IssuerMismatch));
    }
}
