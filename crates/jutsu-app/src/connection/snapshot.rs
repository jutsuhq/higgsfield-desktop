//! What the UI reads of the connection, in one value per query: the shell's state, and the Settings card's rows.

use jutsu_core::ProviderKind;
use serde::Serialize;

use super::card::ConnectionRow;
use super::messages::{SignOutConfirmation, UserMessage};
use super::plate::AccountPlate;
use super::store::{Activity, ConnectionStore};
use super::vault::credential_store_name;

/// What the store is busy with, as the UI reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConnectionActivity {
    Idle,
    ValidatingKey,
    /// The sign-in sheet is up; `url` is the sign-in page once it is known.
    SigningIn {
        url: Option<String>,
    },
}

impl From<Activity> for ConnectionActivity {
    fn from(activity: Activity) -> Self {
        match activity {
            Activity::Idle => Self::Idle,
            Activity::ValidatingKey => Self::ValidatingKey,
            Activity::SigningIn(url) => Self::SigningIn {
                url: url.map(String::from),
            },
        }
    }
}

/// The connection as the window shows it: which kind (none while disconnected), what is going on, and the
/// plate, alert and sign-out confirmation built from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub kind: Option<ProviderKind>,
    /// The stored connection is still being read at launch: the window shows neither onboarding nor the connection.
    pub is_restoring: bool,
    pub activity: ConnectionActivity,
    /// "Higgsfield refused this connection": shown in Settings until the user signs in again or replaces the key.
    pub needs_attention: bool,
    pub alert: Option<UserMessage>,
    pub plate: Option<AccountPlate>,
    pub sign_out: Option<SignOutConfirmation>,
    /// Where the secret is kept, as a sentence names it: "Windows Credential Manager".
    pub credential_store: String,
}

impl ConnectionSnapshot {
    pub fn new(store: &ConnectionStore, low_credit_threshold: i64) -> Self {
        let state = store.state();
        Self {
            kind: state.kind(),
            is_restoring: store.is_restoring(),
            activity: store.activity().into(),
            needs_attention: store.needs_attention(),
            alert: store.alert(),
            plate: AccountPlate::new(&state, low_credit_threshold),
            sign_out: SignOutConfirmation::new(&state),
            credential_store: credential_store_name().to_owned(),
        }
    }
}

/// The Settings › Connection card: its rows, and what the plan's unlimited allowance covers when it covers
/// anything.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetails {
    pub rows: Vec<ConnectionRow>,
    pub unlimited_text: Option<String>,
    /// The account's "Use unlimited generations" switch.
    pub uses_unlimited: bool,
}
