//! The one Higgsfield connection: its state, the store that holds it, the credential vault, the network services
//! behind it, and what the UI shows of it (the account plate, the Settings rows, alerts).

mod card;
mod messages;
mod plate;
mod ports;
mod services;
mod snapshot;
mod state;
mod store;
mod vault;

pub use card::{ConnectionRow, mask, mode_title, unlimited_text};
pub use higgsfield::mcp::oauth::CallbackPage;
pub use messages::{SignOutConfirmation, UserMessage, UserMessageAction};
pub use plate::{AccountPlate, PlanTier, credits_text, grouped, low_credit_warning};
pub use ports::{ConnectionMetadataStore, ConnectionServices, CredentialVault, ShowUrl, VaultError};
pub use services::{
    AccountAuthorization, BringToFront, HiggsfieldConnectionServices, OpenUrl, higgsfield_urls,
};
pub use snapshot::{ConnectionActivity, ConnectionDetails, ConnectionSnapshot};
pub use state::{AccountInfo, ApiKeyForm, ConnectionMetadata, ConnectionState};
pub use store::{Activity, ConnectionStore, CredentialChange};
pub use vault::{KeyringVault, MemoryVault, credential_store_name, credential_store_title};
