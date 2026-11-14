use serde_json::{Value, json};
use url::Url;

use super::metadata::{AuthorizationServerMetadata, GrantType};
use super::token_set::TokenSet;
use crate::core::{send, sorted_json};
use crate::mcp::error::{McpError, OAuthError};
use crate::mcp::http::{is_same_resource, oauth_error};

/// Obtains this app's `client_id` at an authorization server: dynamic client registration (RFC 7591), or the URL
/// of a Client ID Metadata Document when one is configured and the server accepts it.
#[derive(Debug, Clone, Default)]
pub struct ClientRegistration {
    http: reqwest::Client,
    metadata_document_url: Option<Url>,
}

impl ClientRegistration {
    /// Loopback redirects (RFC 8252 §7.3): the browser hands the code to a listener in the app, which works with
    /// every default browser. The ports are fixed so the registered redirect URIs match exactly; the listener
    /// binds the first free one.
    pub const LOOPBACK_PORTS: [u16; 3] = [47613, 51874, 58329];
    pub(crate) const CLIENT_NAME: &'static str = "Jutsu Studio";

    /// `metadata_document_url` is a published Client ID Metadata Document; `None` always registers.
    #[must_use]
    pub fn new(http: reqwest::Client, metadata_document_url: Option<Url>) -> Self {
        Self {
            http,
            metadata_document_url,
        }
    }

    /// Registration is not idempotent, so a `client_id` registered earlier at the same server is reused: pass
    /// the tokens it was stored with as `previous`.
    pub async fn client_id(
        &self,
        server: &AuthorizationServerMetadata,
        previous: Option<&TokenSet>,
    ) -> Result<String, McpError> {
        if server.client_id_metadata_document_supported
            && let Some(document) = &self.metadata_document_url
        {
            return Ok(document.as_str().to_owned());
        }
        // A metadata document client ID is an HTTPS URL; registered IDs never are.
        if let Some(previous) = previous
            && previous
                .issuer
                .as_ref()
                .is_some_and(|issuer| is_same_resource(issuer, &server.issuer))
            && !previous.client_id.starts_with("https://")
        {
            return Ok(previous.client_id.clone());
        }
        let endpoint = server
            .registration_endpoint
            .as_ref()
            .ok_or(OAuthError::RegistrationUnavailable)?;
        let response = send(
            self.http
                .post(endpoint.clone())
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .body(sorted_json(&Self::registration_request())),
        )
        .await?;
        if !response.is_success() {
            return Err(oauth_error(&response));
        }
        serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|body| body.get("client_id")?.as_str().map(str::to_owned))
            .ok_or_else(|| OAuthError::RegistrationUnavailable.into())
    }

    #[must_use]
    pub fn redirect_uri(port: u16) -> String {
        format!("http://127.0.0.1:{port}/callback")
    }

    pub(crate) fn registration_request() -> Value {
        json!({
            "client_name": Self::CLIENT_NAME,
            "redirect_uris": Self::LOOPBACK_PORTS.map(Self::redirect_uri),
            "grant_types": [GrantType::AUTHORIZATION_CODE, GrantType::REFRESH_TOKEN],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })
    }
}
