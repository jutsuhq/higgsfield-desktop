//! The secret of the active connection, and an account's OAuth tokens.

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

use crate::support::codable::Unlabeled;

/// The secret for the active connection. Stored only in the system keychain. Coded as Swift codes the enum:
/// `{"apiKey":{"id":…,"secret":…}}` or `{"oauth":{"_0":{…}}}`. Its `Debug` output redacts the secrets.
#[allow(clippy::large_enum_variant, reason = "one credential is held at a time")]
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "CredentialRepr", from = "CredentialRepr")]
pub enum Credential {
    /// A REST API key.
    ApiKey {
        /// The key's ID.
        id: String,
        /// The key's secret.
        secret: String,
    },
    /// An account signed in with OAuth.
    Oauth(OAuthTokens),
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { id, .. } => write!(f, "Credential.apiKey(id: {id}, secret: <redacted>)"),
            Self::Oauth(tokens) => write!(f, "Credential.oauth({tokens})"),
        }
    }
}

#[allow(clippy::large_enum_variant, reason = "as `Credential`")]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CredentialRepr {
    ApiKey { id: String, secret: String },
    Oauth(Unlabeled<OAuthTokens>),
}

impl From<Credential> for CredentialRepr {
    fn from(credential: Credential) -> Self {
        match credential {
            Credential::ApiKey { id, secret } => Self::ApiKey { id, secret },
            Credential::Oauth(value) => Self::Oauth(Unlabeled { value }),
        }
    }
}

impl From<CredentialRepr> for Credential {
    fn from(repr: CredentialRepr) -> Self {
        match repr {
            CredentialRepr::ApiKey { id, secret } => Self::ApiKey { id, secret },
            CredentialRepr::Oauth(Unlabeled { value }) => Self::Oauth(value),
        }
    }
}

/// An account's OAuth tokens. Its `Debug` and `Display` output redact the tokens.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthTokens {
    /// The access token.
    pub access_token: String,
    /// Absent when the server didn't grant `offline_access`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When the access token expires.
    #[serde(with = "crate::support::date::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Space-separated, as returned by the token endpoint.
    pub scope: String,
    /// The registered client.
    #[serde(rename = "clientID")]
    pub client_id: String,
    /// The authorization server that issued `client_id`; a registered client ID is only valid there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Url>,
    /// From the `id_token`, for display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// From the `id_token`, for display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `id_token`'s `sub`. Not verified: it only tells local jobs of different accounts apart. Absent when the
    /// authorization server sends no subject claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl OAuthTokens {
    /// `GenerationProvider::owner` for this account: its subject, or its email when the server sent none.
    pub fn owner(&self) -> Option<&str> {
        self.subject.as_deref().or(self.email.as_deref())
    }
}

impl fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for OAuthTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let refresh = if self.refresh_token.is_none() {
            "nil"
        } else {
            "<redacted>"
        };
        write!(
            f,
            "OAuthTokens(accessToken: <redacted>, refreshToken: {refresh}, expiresAt: {}, scope: {}, clientID: {})",
            self.expires_at, self.scope, self.client_id
        )
    }
}
