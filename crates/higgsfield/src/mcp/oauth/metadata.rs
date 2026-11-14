use serde::Deserialize;
use url::Url;

use crate::core::is_https;
use crate::mcp::http::is_same_resource;

/// OAuth 2.0 Protected Resource Metadata (RFC 9728).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct ProtectedResourceMetadata {
    pub resource: Url,
    pub authorization_servers: Vec<Url>,
    pub scopes_supported: Option<Vec<String>>,
}

impl ProtectedResourceMetadata {
    /// RFC 9728 §3.3: `resource` must be the resource the metadata was fetched for, or the document is ignored.
    pub(crate) fn describes(&self, expected: &Url) -> bool {
        is_same_resource(&self.resource, expected)
    }
}

/// OAuth 2.0 Authorization Server Metadata (RFC 8414), the fields this client uses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: Url,
    pub authorization_endpoint: Option<Url>,
    pub token_endpoint: Url,
    pub registration_endpoint: Option<Url>,
    /// RFC 7009 token revocation.
    pub revocation_endpoint: Option<Url>,
    pub grant_types_supported: Option<Vec<String>>,
    pub code_challenge_methods_supported: Option<Vec<String>>,
    #[serde(default)]
    pub client_id_metadata_document_supported: bool,
    /// RFC 9207: the server puts `iss` on every authorization response, so one without it is rejected.
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
}

impl AuthorizationServerMetadata {
    /// RFC 8414 §3.3: `issuer` must be the issuer the metadata was fetched for. Every endpoint has to be https,
    /// since the authorization endpoint is opened in the browser and the others receive codes and tokens.
    pub(crate) fn is_issued_by(&self, expected: &Url) -> bool {
        let endpoints = [
            self.authorization_endpoint.as_ref(),
            Some(&self.token_endpoint),
            self.registration_endpoint.as_ref(),
            self.revocation_endpoint.as_ref(),
        ];
        is_same_resource(&self.issuer, expected)
            && is_https(&self.issuer)
            && endpoints.into_iter().flatten().all(is_https)
    }

    /// Authorization code with PKCE S256, the only flow this app signs in with. RFC 8414 §2: an absent grant
    /// list means `authorization_code` and `implicit`.
    #[must_use]
    pub fn supports_authorization_code(&self) -> bool {
        self.authorization_endpoint.is_some()
            && self
                .grant_types_supported
                .as_ref()
                .is_none_or(|grants| grants.iter().any(|grant| grant == GrantType::AUTHORIZATION_CODE))
            && self
                .code_challenge_methods_supported
                .as_ref()
                .is_some_and(|methods| methods.iter().any(|method| method == "S256"))
    }
}

/// `grant_type` values this client sends.
pub(crate) struct GrantType;

impl GrantType {
    pub const AUTHORIZATION_CODE: &'static str = "authorization_code";
    pub const REFRESH_TOKEN: &'static str = "refresh_token";
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLERK: &str = "https://clerk.higgsfield.ai";
    const CLERK_METADATA: &str = include_str!("../../../tests/fixtures/oauth/clerk-metadata.json");

    fn server(json: &str) -> AuthorizationServerMetadata {
        serde_json::from_str(json).unwrap()
    }

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn non_https_endpoint_is_rejected() {
        for endpoint in [
            "https://clerk.higgsfield.ai/oauth/authorize",
            "https://clerk.higgsfield.ai/oauth/token",
            "https://clerk.higgsfield.ai/oauth/register",
            "https://clerk.higgsfield.ai/oauth/token/revoke",
        ] {
            let downgraded = CLERK_METADATA.replace(
                &format!("\"{endpoint}\""),
                "\"file:///Applications/Calculator.app\"",
            );
            assert_ne!(downgraded, CLERK_METADATA);
            assert!(!server(&downgraded).is_issued_by(&url(CLERK)), "{endpoint}");
        }
    }
}
