use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

/// What a token endpoint granted, plus the account the `id_token` names. Stored by the app as JSON with these
/// camelCase keys; `Debug` never shows the tokens.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSet {
    pub access_token: String,
    /// Absent when the server didn't grant `offline_access`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Space-separated, as returned by the token endpoint.
    pub scope: String,
    #[serde(rename = "clientID")]
    pub client_id: String,
    /// The authorization server that issued `client_id`; a registered client ID is only valid there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Url>,
    /// From the `id_token`, for display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `id_token`'s `sub`. Not verified: it only tells local jobs of different accounts apart. Absent when
    /// the authorization server sends no subject claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl TokenSet {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at: OffsetDateTime,
        scope: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token,
            expires_at,
            scope: scope.into(),
            client_id: client_id.into(),
            issuer: None,
            email: None,
            name: None,
            subject: None,
        }
    }
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let refresh = if self.refresh_token.is_some() {
            "Some(<redacted>)"
        } else {
            "None"
        };
        formatter
            .debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &format_args!("{refresh}"))
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_tokens() {
        let tokens = TokenSet::new(
            "secret-access",
            Some("secret-refresh".into()),
            OffsetDateTime::UNIX_EPOCH,
            "openid",
            "client-1",
        );
        let printed = format!("{tokens:?}");
        assert!(!printed.contains("secret"), "{printed}");
        assert!(printed.contains("client-1"));
    }

    #[test]
    fn round_trips_through_json_with_camel_case_keys() {
        let mut tokens = TokenSet::new("a", None, OffsetDateTime::UNIX_EPOCH, "openid", "client-1");
        tokens.issuer = Some(Url::parse("https://clerk.higgsfield.ai").unwrap());
        let json = serde_json::to_value(&tokens).unwrap();
        assert_eq!(json["clientID"], "client-1");
        assert_eq!(json["accessToken"], "a");
        assert_eq!(json["expiresAt"], "1970-01-01T00:00:00Z");
        assert!(json.get("refreshToken").is_none());
        assert_eq!(serde_json::from_value::<TokenSet>(json).unwrap(), tokens);
    }
}
