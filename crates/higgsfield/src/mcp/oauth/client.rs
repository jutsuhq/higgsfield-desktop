use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use time::OffsetDateTime;
use url::Url;

use super::id_token::IdTokenClaims;
use super::metadata::{AuthorizationServerMetadata, GrantType};
use super::token_set::TokenSet;
use crate::core::HttpResponse;
use crate::mcp::error::{McpError, OAuthError};
use crate::mcp::http::{oauth_error, post_form};

/// The clock a client stamps token expiry with.
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;

/// This app as a public OAuth client of one authorization server, for one protected resource. Owns the token
/// endpoint; the authorizers build on it.
#[derive(Clone)]
pub struct OAuthClient {
    pub server: AuthorizationServerMetadata,
    pub client_id: String,
    /// Sent as `resource` (RFC 8707) so tokens are bound to the MCP server.
    pub resource: Url,
    pub scope: String,
    http: reqwest::Client,
    now: Clock,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthClient")
            .field("issuer", &self.server.issuer.as_str())
            .field("client_id", &self.client_id)
            .field("resource", &self.resource.as_str())
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl OAuthClient {
    pub const DEFAULT_SCOPE: &'static str = "openid email offline_access";

    pub fn new(server: AuthorizationServerMetadata, client_id: impl Into<String>, resource: Url) -> Self {
        Self {
            server,
            client_id: client_id.into(),
            resource,
            scope: Self::DEFAULT_SCOPE.to_owned(),
            http: reqwest::Client::new(),
            now: Arc::new(OffsetDateTime::now_utc),
        }
    }

    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    #[must_use]
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    #[must_use]
    pub fn with_clock(mut self, now: Clock) -> Self {
        self.now = now;
        self
    }

    /// Exchanges a refresh token. The server may omit a new refresh token; the old one then stays valid. Fails
    /// with `OAuthError::ReauthenticationRequired` only when the token endpoint refuses the grant (a 400 or 401,
    /// or an `invalid_grant` or `invalid_client` error); any other answer is `HiggsfieldError::Server`.
    pub async fn refresh(&self, tokens: &TokenSet) -> Result<TokenSet, McpError> {
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or(OAuthError::ReauthenticationRequired)?;
        let response = self
            .post_token_request(BTreeMap::from([
                ("grant_type".to_owned(), GrantType::REFRESH_TOKEN.to_owned()),
                ("refresh_token".to_owned(), refresh_token.clone()),
                ("resource".to_owned(), self.resource.as_str().to_owned()),
            ]))
            .await?;
        if !response.is_success() {
            return Err(refresh_error(&response));
        }
        let mut refreshed = self.token_set(&response)?;
        refreshed.refresh_token = refreshed.refresh_token.or(Some(refresh_token));
        refreshed.email = refreshed.email.or_else(|| tokens.email.clone());
        refreshed.name = refreshed.name.or_else(|| tokens.name.clone());
        refreshed.subject = refreshed.subject.or_else(|| tokens.subject.clone());
        Ok(refreshed)
    }

    /// Revokes the refresh token (RFC 7009), which also ends its access tokens at most servers. Without a refresh
    /// token, revokes the access token. `OAuthError::UnsupportedFlow` when the server has no revocation endpoint.
    pub async fn revoke(&self, tokens: &TokenSet) -> Result<(), McpError> {
        let endpoint = self
            .server
            .revocation_endpoint
            .as_ref()
            .ok_or(OAuthError::UnsupportedFlow)?;
        let (token, hint) = match &tokens.refresh_token {
            Some(refresh) => (refresh.clone(), "refresh_token"),
            None => (tokens.access_token.clone(), "access_token"),
        };
        let parameters = BTreeMap::from([
            ("token".to_owned(), token),
            ("token_type_hint".to_owned(), hint.to_owned()),
            ("client_id".to_owned(), self.client_id.clone()),
        ]);
        let response = post_form(&self.http, endpoint, &parameters).await?;
        if response.is_success() {
            Ok(())
        } else {
            Err(oauth_error(&response))
        }
    }

    /// POSTs `parameters` plus `client_id` to the token endpoint.
    pub(crate) async fn request_tokens(
        &self,
        parameters: BTreeMap<String, String>,
    ) -> Result<TokenSet, McpError> {
        let response = self.post_token_request(parameters).await?;
        if !response.is_success() {
            return Err(oauth_error(&response));
        }
        self.token_set(&response)
    }

    async fn post_token_request(
        &self,
        mut parameters: BTreeMap<String, String>,
    ) -> Result<HttpResponse, McpError> {
        parameters
            .entry("client_id".to_owned())
            .or_insert_with(|| self.client_id.clone());
        Ok(post_form(&self.http, &self.server.token_endpoint, &parameters).await?)
    }

    /// An unreadable 2xx body is `HiggsfieldError::Server` with its status.
    fn token_set(&self, response: &HttpResponse) -> Result<TokenSet, McpError> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<f64>,
            scope: Option<String>,
            id_token: Option<String>,
        }
        let body: TokenResponse =
            serde_json::from_slice(&response.body).map_err(|_| McpError::server(response.status))?;
        let account = body
            .id_token
            .as_deref()
            .and_then(IdTokenClaims::from_jwt)
            .unwrap_or_default();
        // note: a missing expires_in is treated as one hour; refresh on 401 covers a shorter real lifetime.
        let lifetime = time::Duration::saturating_seconds_f64(body.expires_in.unwrap_or(3600.0));
        Ok(TokenSet {
            access_token: body.access_token,
            refresh_token: body.refresh_token,
            expires_at: (self.now)().saturating_add(lifetime),
            scope: body.scope.unwrap_or_else(|| self.scope.clone()),
            client_id: self.client_id.clone(),
            issuer: Some(self.server.issuer.clone()),
            email: account.email,
            name: account.name,
            subject: account.subject,
        })
    }
}

/// RFC 6749 §5.2: a refused grant answers 400, or 401 for `invalid_client`.
fn refresh_error(response: &HttpResponse) -> McpError {
    if response.status == 400 || response.status == 401 {
        return OAuthError::ReauthenticationRequired.into();
    }
    if let McpError::OAuth(error) = oauth_error(response)
        && error.refuses_refresh()
    {
        return OAuthError::ReauthenticationRequired.into();
    }
    McpError::server(response.status)
}
