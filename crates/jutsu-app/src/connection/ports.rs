//! What the connection store reaches the credential store and the network through (`ConnectionPorts.swift`), so
//! its state machine is tested with fakes.

use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::{Credential, OAuthTokens, PortError};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::state::ConnectionMetadata;

/// Why the credential store failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    /// The store refused or failed; the description says why.
    #[error("{0}")]
    Store(String),
    /// What is stored isn't a credential this app wrote.
    #[error("The stored credential can't be read: {0}")]
    Decoding(String),
}

/// Where the credential lives: the system's credential store, or memory in tests. The signatures match the macOS
/// app's `SecretStore`, so an adapter only forwards.
pub trait CredentialVault: Send + Sync {
    fn save_data(&self, data: &[u8]) -> Result<(), VaultError>;
    fn load_data(&self) -> Result<Option<Vec<u8>>, VaultError>;
    /// Deleting when nothing is stored is not an error.
    fn delete(&self) -> Result<(), VaultError>;

    /// The credential as the JSON the macOS app stores.
    fn save_credential(&self, credential: &Credential) -> Result<(), VaultError> {
        let data = serde_json::to_vec(credential).map_err(|error| VaultError::Decoding(error.to_string()))?;
        self.save_data(&data)
    }

    fn load_credential(&self) -> Result<Option<Credential>, VaultError> {
        let Some(data) = self.load_data()? else {
            return Ok(None);
        };
        serde_json::from_slice(&data)
            .map(Some)
            .map_err(|error| VaultError::Decoding(error.to_string()))
    }
}

/// Persists `ConnectionMetadata`.
pub trait ConnectionMetadataStore: Send + Sync {
    fn load(&self) -> Option<ConnectionMetadata>;
    fn save(&self, metadata: &ConnectionMetadata);
    fn clear(&self);
}

/// Shows the sign-in page's URL once it is known.
pub type ShowUrl = Arc<dyn Fn(Url) + Send + Sync>;

/// The network side of connecting. The app implements it with the `higgsfield` crate and maps OAuth failures to
/// `SignInError`, so the store never sees either.
#[async_trait]
pub trait ConnectionServices: Send + Sync {
    /// `false` when the key was refused; other failures are errors.
    async fn validate_api_key(&self, id: &str, secret: &str) -> Result<bool, PortError>;
    /// OAuth with PKCE in the default browser. Calls `show` with the sign-in page once it is known, so the user can
    /// open it again or copy it when the browser didn't come up. Ends with `PortError::Cancelled` once `cancel` is
    /// cancelled. `previous` is the stored sign-in, whose registered client ID can be reused.
    async fn sign_in_with_browser(
        &self,
        previous: Option<OAuthTokens>,
        show: ShowUrl,
        cancel: CancellationToken,
    ) -> Result<OAuthTokens, PortError>;
    /// Revokes `tokens` at the server that issued them. Best effort: failures, offline included, are ignored.
    async fn revoke(&self, tokens: OAuthTokens);
}
