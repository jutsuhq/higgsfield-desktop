//! Provider failures in terms the engine acts on.

use time::Duration;

/// Provider failures in terms the engine acts on; each provider maps its transport errors to these.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ProviderError {
    /// The credential was refused.
    #[error("unauthorized")]
    Unauthorized,
    /// The account can't pay.
    #[error("insufficientFunds")]
    InsufficientFunds,
    /// The account is at its limit: nothing started, and the same request can be sent again. `retry_after` is the
    /// wait the server asked for, when it said.
    #[error("concurrencyLimit")]
    ConcurrencyLimit {
        /// The wait the server asked for.
        retry_after: Option<Duration>,
    },
    /// The request was refused as invalid.
    #[error("{message}")]
    Validation {
        /// The field, when the server named one.
        field: Option<String>,
        /// What the server said.
        message: String,
    },
    /// The model isn't available.
    #[error("modelUnavailable")]
    ModelUnavailable,
    /// The server answered with this status.
    #[error("server({status})")]
    Server {
        /// The HTTP status.
        status: i64,
    },
    /// The network failed.
    #[error("network")]
    Network,
    /// Refused by content moderation.
    #[error("rejected")]
    Rejected,
    /// A submit failed after it may have reached the server, and resending it could charge twice.
    #[error("outcomeUnknown")]
    OutcomeUnknown,
}
