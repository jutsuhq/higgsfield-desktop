//! The one connection the app has, as the UI shows it (`ConnectionState.swift`). Never carries a secret.

use jutsu_core::{Credential, ProviderKind};
use serde::{Deserialize, Serialize};

/// The one connection the app has, as the UI shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    #[default]
    None,
    ApiKey {
        key_id: String,
    },
    Account(AccountInfo),
}

impl ConnectionState {
    pub fn kind(&self) -> Option<ProviderKind> {
        match self {
            Self::None => None,
            Self::ApiKey { .. } => Some(ProviderKind::ApiKey),
            Self::Account(_) => Some(ProviderKind::Account),
        }
    }

    pub fn is_connected(&self) -> bool {
        *self != Self::None
    }

    /// Rebuilds the state from what was persisted at connect time.
    pub fn from_metadata(metadata: Option<&ConnectionMetadata>) -> Self {
        match metadata {
            None => Self::None,
            Some(metadata) => match metadata.kind {
                ProviderKind::ApiKey => Self::ApiKey {
                    key_id: metadata.display_name.clone().unwrap_or_default(),
                },
                ProviderKind::Account => Self::Account(AccountInfo {
                    email: metadata.display_name.clone(),
                    name: metadata.name.clone(),
                    plan: metadata.plan.clone(),
                    credits: None,
                }),
            },
        }
    }

    /// What is persisted for this state; `None` when not connected.
    pub fn metadata(&self) -> Option<ConnectionMetadata> {
        match self {
            Self::None => None,
            Self::ApiKey { key_id } => Some(ConnectionMetadata {
                kind: ProviderKind::ApiKey,
                display_name: Some(key_id.clone()),
                plan: None,
                name: None,
            }),
            Self::Account(info) => Some(ConnectionMetadata {
                kind: ProviderKind::Account,
                display_name: info.email.clone(),
                plan: info.plan.clone(),
                name: info.name.clone(),
            }),
        }
    }

    pub fn account_info(&self) -> Option<&AccountInfo> {
        match self {
            Self::Account(info) => Some(info),
            _ => None,
        }
    }
}

/// What is known about a signed-in Higgsfield account. E-mail and name come from the sign-in's `id_token` when the
/// server sends one; plan and credits stay `None` until a provider call fills them in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AccountInfo {
    pub email: Option<String>,
    pub name: Option<String>,
    pub plan: Option<String>,
    /// Not persisted: credits change with every job.
    pub credits: Option<i64>,
}

/// The non-secret part of a connection, kept with the preferences so the UI can show it without the credential
/// store. Coded as the Swift `Codable` struct.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionMetadata {
    pub kind: ProviderKind,
    /// Key ID for an API key, e-mail for an account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The account holder's name; `None` for an API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The connection kind of a credential.
pub(crate) fn credential_kind(credential: &Credential) -> ProviderKind {
    match credential {
        Credential::ApiKey { .. } => ProviderKind::ApiKey,
        Credential::Oauth(_) => ProviderKind::Account,
    }
}

/// Whether `metadata` describes `credential`: the same kind and, where both name one, the same key ID or e-mail.
/// Keeps a credential written for another connection from showing under this one's name.
pub(crate) fn credential_matches(credential: &Credential, metadata: &ConnectionMetadata) -> bool {
    if credential_kind(credential) != metadata.kind {
        return false;
    }
    match credential {
        Credential::ApiKey { id, .. } => metadata.display_name.as_deref() == Some(id.as_str()),
        Credential::Oauth(tokens) => match (&tokens.email, &metadata.display_name) {
            (Some(email), Some(shown)) => email == shown,
            _ => true,
        },
    }
}

/// The "Use an API key" card (`APIKeyForm.swift`). Pasted keys often carry a trailing newline, so both fields
/// are trimmed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ApiKeyForm {
    pub key_id: String,
    pub secret: String,
}

impl ApiKeyForm {
    pub fn new(key_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            key_id: key_id.into(),
            secret: secret.into(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.credential().is_some()
    }

    /// `None` until both fields have something besides whitespace.
    pub fn credential(&self) -> Option<Credential> {
        let id = self.key_id.trim();
        let secret = self.secret.trim();
        if id.is_empty() || secret.is_empty() {
            return None;
        }
        Some(Credential::ApiKey {
            id: id.to_owned(),
            secret: secret.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_fields_are_incomplete() {
        for (key_id, secret) in [("", "s"), ("kid", ""), (" ", "s"), ("kid", " \n")] {
            let form = ApiKeyForm::new(key_id, secret);
            assert_eq!(form.credential(), None);
            assert!(!form.is_complete());
        }
    }

    #[test]
    fn metadata_is_coded_as_the_swift_struct() {
        let metadata = ConnectionMetadata {
            kind: ProviderKind::ApiKey,
            display_name: Some("kid".into()),
            plan: None,
            name: None,
        };
        assert_eq!(
            serde_json::to_value(&metadata).unwrap(),
            serde_json::json!({ "kind": "apiKey", "displayName": "kid" })
        );
    }
}
