//! Alerts and confirmations (`UserMessage.swift`): a short title, one sentence of detail, and where to go next.

use jutsu_core::{FailureReason, PortError, ProviderError, ProviderKind, SignInError};
use serde::Serialize;

use super::state::ConnectionState;
use super::vault::credential_store_name;

/// Where an alert leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UserMessageAction {
    OpenSettings,
    OpenConsole,
}

/// An alert: short title, one sentence of detail, and where to go next.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
pub struct UserMessage {
    pub title: String,
    pub message: String,
    pub action: Option<UserMessageAction>,
}

impl UserMessage {
    pub fn new(title: &str, message: impl Into<String>, action: Option<UserMessageAction>) -> Self {
        Self {
            title: title.to_owned(),
            message: message.into(),
            action,
        }
    }

    pub fn incomplete_key() -> Self {
        Self::new(
            "Enter both the key ID and the secret",
            "You find both in the Higgsfield console.",
            None,
        )
    }

    pub fn invalid_key() -> Self {
        Self::new(
            "Key is invalid",
            "Higgsfield refused this key ID and secret. Check both and try again.",
            Some(UserMessageAction::OpenConsole),
        )
    }

    pub fn not_revoked() -> Self {
        Self::new(
            "Signed out on this computer only",
            "Jutsu Studio couldn't read the sign-in to revoke it, so it stays valid on Higgsfield until it expires.",
            None,
        )
    }

    /// "Something went wrong" with `description`.
    pub fn failure(description: impl Into<String>) -> Self {
        Self::new("Something went wrong", description, None)
    }

    /// The message for `error`, or `None` when the user shouldn't see one (they canceled, or the engine handles it
    /// silently). `kind` is the connection the error came from, when known.
    pub fn from_error(error: &PortError, kind: Option<ProviderKind>) -> Option<Self> {
        match error {
            PortError::Cancelled => None,
            PortError::Provider(error) => Self::provider(error, kind),
            PortError::SignIn(error) => Some(Self::sign_in(error)),
            PortError::Failure(reason) => Some(Self::failure(failure_text(reason))),
            PortError::Transcription(error) => Some(Self::failure(error.to_string())),
            PortError::Other(description) => Some(Self::failure(description.clone())),
        }
    }

    fn provider(error: &ProviderError, kind: Option<ProviderKind>) -> Option<Self> {
        Some(match error {
            ProviderError::Unauthorized => Self::refused_credential(kind),
            ProviderError::InsufficientFunds => {
                if kind == Some(ProviderKind::Account) {
                    Self::new(
                        "Not enough credits",
                        "Your plan has no credits left for this job.",
                        None,
                    )
                } else {
                    Self::new(
                        "Not enough balance",
                        "Top up your API balance in the Higgsfield console.",
                        Some(UserMessageAction::OpenConsole),
                    )
                }
            }
            ProviderError::ConcurrencyLimit { .. } => return None,
            ProviderError::Validation { field, message } => Self::new(
                "Higgsfield refused the request",
                field
                    .as_ref()
                    .map_or_else(|| message.clone(), |field| format!("{field}: {message}")),
                None,
            ),
            ProviderError::ModelUnavailable => Self::new(
                "Model temporarily unavailable",
                "Jutsu Studio tries again later by itself.",
                None,
            ),
            ProviderError::Server { status } => Self::new(
                "Higgsfield is having trouble",
                format!("The server answered {status}. Try again in a few minutes."),
                None,
            ),
            ProviderError::Network => Self::new(
                "Can't reach Higgsfield",
                "Check your internet connection and try again.",
                None,
            ),
            ProviderError::Rejected => {
                Self::new("Rejected by content moderation", "You were not charged.", None)
            }
            ProviderError::OutcomeUnknown => Self::new(
                "The job may have started",
                "Check your generations on Higgsfield before trying again, or it may be charged twice.",
                None,
            ),
        })
    }

    /// A refused key (wrong, expired or disabled) or a sign-in that can't be refreshed.
    fn refused_credential(kind: Option<ProviderKind>) -> Self {
        if kind == Some(ProviderKind::Account) {
            Self::sign_in_again()
        } else {
            Self::new(
                "API key is invalid",
                "Higgsfield refused the key: it is wrong, expired or disabled. Check it in Settings.",
                Some(UserMessageAction::OpenSettings),
            )
        }
    }

    fn sign_in_again() -> Self {
        Self::new(
            "Sign in again",
            "Your Higgsfield sign-in has expired.",
            Some(UserMessageAction::OpenSettings),
        )
    }

    fn sign_in(error: &SignInError) -> Self {
        match error {
            SignInError::Unavailable => Self::new(
                "Sign-in isn't available right now",
                "Higgsfield's sign-in server didn't answer as expected. Try again later or use an API key.",
                None,
            ),
            SignInError::FlowUnavailable => Self::new(
                "This sign-in method isn't available",
                "Higgsfield didn't accept Jutsu Studio's sign-in. Connect with an API key instead.",
                None,
            ),
            SignInError::Denied => Self::new(
                "Access was denied",
                "Jutsu Studio wasn't allowed to use your account.",
                None,
            ),
            SignInError::TimedOut => Self::new(
                "Sign-in timed out",
                "Jutsu Studio stopped waiting for the browser. Try signing in again.",
                None,
            ),
            SignInError::InvalidResponse => Self::new(
                "Sign-in failed",
                "Higgsfield's answer didn't check out. Try again.",
                None,
            ),
            SignInError::ReauthenticationRequired => Self::sign_in_again(),
            SignInError::Server(description) => Self::new(
                "Sign-in failed",
                description
                    .clone()
                    .unwrap_or_else(|| "Higgsfield refused the sign-in.".to_owned()),
                None,
            ),
        }
    }
}

/// `FailureReason.text`, as far as this feature needs it: the reason's own description.
// note: the full `FailureReason+Text` table is ported with the Queue, which shows every reason.
fn failure_text(reason: &FailureReason) -> String {
    reason.to_string()
}

/// The confirmation shown before the connection is deleted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SignOutConfirmation {
    pub title: String,
    pub message: String,
    pub confirm_title: String,
}

impl SignOutConfirmation {
    /// `None` when there is nothing to sign out of.
    pub fn new(state: &ConnectionState) -> Option<Self> {
        let consequence = "Queued jobs pause until you connect again.";
        let store = credential_store_name();
        match state {
            ConnectionState::None => None,
            ConnectionState::ApiKey { .. } => Some(Self {
                title: "Remove the API key?".to_owned(),
                message: format!("The key is deleted from {store}. {consequence}"),
                confirm_title: "Remove".to_owned(),
            }),
            ConnectionState::Account(_) => Some(Self {
                title: "Sign out of Higgsfield?".to_owned(),
                message: format!("Your sign-in is deleted from {store}. {consequence}"),
                confirm_title: "Sign Out".to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::AccountInfo;

    fn message(error: impl Into<PortError>, kind: Option<ProviderKind>) -> Option<UserMessage> {
        UserMessage::from_error(&error.into(), kind)
    }

    #[test]
    fn cancellation_and_concurrency_are_silent() {
        assert_eq!(UserMessage::from_error(&PortError::Cancelled, None), None);
        assert_eq!(
            message(ProviderError::ConcurrencyLimit { retry_after: None }, None),
            None
        );
    }

    #[test]
    fn unauthorized_depends_on_the_connection() {
        let key = message(ProviderError::Unauthorized, Some(ProviderKind::ApiKey)).unwrap();
        assert_eq!(key.title, "API key is invalid");
        let unknown = message(ProviderError::Unauthorized, None).unwrap();
        assert_eq!(unknown.title, "API key is invalid");
        assert!(unknown.message.contains("expired or disabled"));
        assert_eq!(unknown.action, Some(UserMessageAction::OpenSettings));
        let account = message(ProviderError::Unauthorized, Some(ProviderKind::Account)).unwrap();
        assert_eq!(account.title, "Sign in again");
    }

    #[test]
    fn insufficient_funds_links_the_console_for_api_keys() {
        let key = message(ProviderError::InsufficientFunds, Some(ProviderKind::ApiKey)).unwrap();
        assert_eq!(key.title, "Not enough balance");
        assert_eq!(key.action, Some(UserMessageAction::OpenConsole));
        let account = message(ProviderError::InsufficientFunds, Some(ProviderKind::Account)).unwrap();
        assert_eq!(account.title, "Not enough credits");
    }

    #[test]
    fn validation_names_the_field() {
        let named = ProviderError::Validation {
            field: Some("prompt".into()),
            message: "too long".into(),
        };
        assert_eq!(message(named, None).unwrap().message, "prompt: too long");
        let bare = ProviderError::Validation {
            field: None,
            message: "bad".into(),
        };
        assert_eq!(message(bare, None).unwrap().message, "bad");
    }

    #[test]
    fn fixed_texts_from_features() {
        assert_eq!(
            message(ProviderError::ModelUnavailable, None).unwrap().title,
            "Model temporarily unavailable"
        );
        let rejected = message(ProviderError::Rejected, None).unwrap();
        assert_eq!(rejected.title, "Rejected by content moderation");
        assert_eq!(rejected.message, "You were not charged.");
        assert!(
            message(ProviderError::Server { status: 502 }, None)
                .unwrap()
                .message
                .contains("502")
        );
    }

    #[test]
    fn server_description_is_shown() {
        assert_eq!(
            message(SignInError::Server(Some("Client not allowed".into())), None)
                .unwrap()
                .message,
            "Client not allowed"
        );
    }

    #[test]
    fn sign_out_texts_match_the_connection() {
        assert_eq!(SignOutConfirmation::new(&ConnectionState::None), None);
        let key = SignOutConfirmation::new(&ConnectionState::ApiKey { key_id: "kid".into() }).unwrap();
        assert_eq!(key.title, "Remove the API key?");
        assert_eq!(key.confirm_title, "Remove");
        assert!(key.message.contains(credential_store_name()));
        let account = SignOutConfirmation::new(&ConnectionState::Account(AccountInfo::default())).unwrap();
        assert_eq!(account.title, "Sign out of Higgsfield?");
        assert_eq!(account.confirm_title, "Sign Out");
    }
}
