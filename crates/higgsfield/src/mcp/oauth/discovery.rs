use std::collections::HashMap;

use serde::de::DeserializeOwned;
use url::Url;

use super::client::OAuthClient;
use super::metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata};
use super::token_set::TokenSet;
use crate::core::{HiggsfieldError, is_https, send};
use crate::mcp::error::{McpError, OAuthError};
use crate::mcp::http::is_same_resource;

/// Everything sign-in needs to know about an MCP server, read from its metadata documents.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiscoveredAuthorization {
    pub resource: ProtectedResourceMetadata,
    /// In the order the resource lists them; servers whose metadata couldn't be read are left out.
    pub servers: Vec<AuthorizationServerMetadata>,
}

impl DiscoveredAuthorization {
    /// The first readable server that offers the authorization code flow with PKCE.
    #[must_use]
    pub fn authorization_code_server(&self) -> Option<&AuthorizationServerMetadata> {
        self.servers
            .iter()
            .find(|server| server.supports_authorization_code())
    }

    /// The client that `tokens` were issued to, for refreshing or revoking them. Refresh tokens go only to the
    /// server that issued them: when it is no longer advertised, or `tokens` don't name it, the user has to sign
    /// in again.
    pub fn client_issuing(&self, tokens: &TokenSet, http: reqwest::Client) -> Result<OAuthClient, McpError> {
        let server = tokens
            .issuer
            .as_ref()
            .and_then(|issuer| {
                self.servers
                    .iter()
                    .find(|server| is_same_resource(&server.issuer, issuer))
            })
            .ok_or(OAuthError::ReauthenticationRequired)?;
        Ok(OAuthClient::new(
            server.clone(),
            tokens.client_id.clone(),
            self.resource.resource.clone(),
        )
        .with_http_client(http))
    }
}

/// Reads protected resource metadata (RFC 9728) and then each authorization server's metadata (RFC 8414,
/// falling back to `OpenID` Connect discovery). No endpoint is hard-coded.
#[derive(Debug, Clone, Default)]
pub struct OAuthDiscovery {
    http: reqwest::Client,
}

impl OAuthDiscovery {
    #[must_use]
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// `www_authenticate` is the header of a `401` from the MCP endpoint, when there was one; its
    /// `resource_metadata` URL is tried first.
    pub async fn discover(
        &self,
        mcp_url: &Url,
        www_authenticate: Option<&str>,
    ) -> Result<DiscoveredAuthorization, McpError> {
        let mut candidates = resource_metadata_urls(mcp_url);
        if let Some(hinted) = www_authenticate
            .and_then(|header| challenge_parameters(header).remove("resource_metadata"))
            .and_then(|hinted| Url::parse(&hinted).ok())
            .filter(is_https)
        {
            candidates.insert(0, hinted);
        }
        let resource = self
            .first_document::<ProtectedResourceMetadata>(&candidates, |document| document.describes(mcp_url))
            .await?
            .ok_or(OAuthError::DiscoveryFailed)?;
        let mut servers = Vec::new();
        let mut unreachable = false;
        for issuer in resource
            .authorization_servers
            .iter()
            .filter(|issuer| is_https(issuer))
        {
            // One advertised server that can't be reached must not hide another that answers.
            match self
                .first_document::<AuthorizationServerMetadata>(
                    &authorization_server_metadata_urls(issuer),
                    |document| document.is_issued_by(issuer),
                )
                .await
            {
                Ok(Some(server)) => servers.push(server),
                Ok(None) => {}
                Err(_) => unreachable = true,
            }
        }
        if servers.is_empty() {
            return Err(if unreachable {
                HiggsfieldError::Network.into()
            } else {
                OAuthError::DiscoveryFailed.into()
            });
        }
        Ok(DiscoveredAuthorization { resource, servers })
    }

    /// The first URL that answers 2xx with a decodable document that passes `is_valid`. URLs that can't be
    /// reached are skipped. `None` when none has such a document; `Network` when none answered at all.
    async fn first_document<T: DeserializeOwned>(
        &self,
        urls: &[Url],
        is_valid: impl Fn(&T) -> bool,
    ) -> Result<Option<T>, HiggsfieldError> {
        let mut answered = false;
        for url in urls {
            let Ok(response) = send(self.http.get(url.clone()).header("Accept", "application/json")).await
            else {
                continue;
            };
            answered = true;
            if response.is_success()
                && let Ok(document) = serde_json::from_slice::<T>(&response.body)
                && is_valid(&document)
            {
                return Ok(Some(document));
            }
        }
        if answered {
            Ok(None)
        } else {
            Err(HiggsfieldError::Network)
        }
    }
}

/// RFC 9728 §3.1: the well-known suffix goes between host and path, then the bare root as fallback.
pub(crate) fn resource_metadata_urls(resource: &Url) -> Vec<Url> {
    well_known_urls(resource, "oauth-protected-resource")
}

/// RFC 8414 §3.1 path insertion, then `OpenID` Connect's appended `openid-configuration`.
pub(crate) fn authorization_server_metadata_urls(issuer: &Url) -> Vec<Url> {
    let mut urls = well_known_urls(issuer, "oauth-authorization-server");
    urls.push(with_path(
        issuer,
        &format!("{}/.well-known/openid-configuration", trimmed_path(issuer)),
    ));
    urls
}

/// Parameters of a `Bearer` challenge: `Bearer realm="x", resource_metadata="https://..."`.
pub(crate) fn challenge_parameters(header: &str) -> HashMap<String, String> {
    let mut parameters = HashMap::new();
    let mut rest = header.split_once(' ').map_or(header, |(_, rest)| rest);
    while let Some((key, after)) = rest.split_once('=') {
        let key = key.trim_matches([' ', ',']).to_lowercase();
        let value;
        if let Some(quoted) = after.strip_prefix('"')
            && let Some((inside, tail)) = quoted.split_once('"')
        {
            value = inside;
            rest = tail;
        } else {
            let end = after.find(',').unwrap_or(after.len());
            value = &after[..end];
            rest = &after[end..];
        }
        parameters.insert(key, value.trim().to_owned());
    }
    parameters
}

fn well_known_urls(resource: &Url, suffix: &str) -> Vec<Url> {
    let path = trimmed_path(resource);
    let root = format!("/.well-known/{suffix}");
    let paths = if path.is_empty() {
        vec![root]
    } else {
        vec![format!("{root}{path}"), root]
    };
    paths.iter().map(|path| with_path(resource, path)).collect()
}

fn trimmed_path(url: &Url) -> &str {
    let path = url.path();
    path.strip_suffix('/').unwrap_or(path)
}

/// `base`'s scheme, host and port with `path`.
fn with_path(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    const MCP_URL: &str = "https://mcp.higgsfield.ai/mcp";
    const RESOURCE_METADATA_URL: &str = "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp";
    const CLERK: &str = "https://clerk.higgsfield.ai";

    #[test]
    fn www_authenticate_parameters_are_parsed() {
        let header = format!(
            r#"Bearer resource_metadata="{RESOURCE_METADATA_URL}", scope="openid email offline_access""#
        );
        let parameters = challenge_parameters(&header);
        assert_eq!(parameters["resource_metadata"], RESOURCE_METADATA_URL);
        assert_eq!(parameters["scope"], "openid email offline_access");
        assert_eq!(
            challenge_parameters(r#"Bearer realm=mcp, error="invalid_token""#)["realm"],
            "mcp"
        );
    }

    #[test]
    fn well_known_urls_follow_rfc_9728_and_rfc_8414() {
        let resource: Vec<String> = resource_metadata_urls(&Url::parse(MCP_URL).unwrap())
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            resource,
            [
                RESOURCE_METADATA_URL,
                "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource"
            ]
        );
        let issuer: Vec<String> = authorization_server_metadata_urls(&Url::parse(CLERK).unwrap())
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            issuer,
            [
                format!("{CLERK}/.well-known/oauth-authorization-server"),
                format!("{CLERK}/.well-known/openid-configuration")
            ]
        );
    }
}
