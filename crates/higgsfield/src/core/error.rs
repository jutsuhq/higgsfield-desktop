use url::Url;

/// A failed call to either Higgsfield connection, in terms a caller can act on.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HiggsfieldError {
    /// The credential was refused: a wrong or revoked API key, or a sign-in that can't be refreshed.
    #[error("the credential was refused")]
    Unauthorized,
    #[error("the account has too few credits")]
    InsufficientFunds,
    /// The account is at its limit (a full generation queue, `429`): nothing was created or charged, and the same
    /// request can be sent again. `retry_after` is the wait the server asked for, in seconds, when it said.
    #[error("the account is at its limit")]
    ConcurrencyLimit { retry_after: Option<f64> },
    /// The request was refused as it is; `field` names the offending parameter when the server did.
    #[error("the request was refused: {message}")]
    Validation { field: Option<String>, message: String },
    #[error("the model is unavailable")]
    ModelUnavailable,
    /// The account connection doesn't offer a tool this call needs, by name and arguments.
    #[error("the tool {0} is unavailable")]
    ToolUnavailable(String),
    /// An answer of this status the client can't read or act on, including an unreadable 2xx body.
    #[error("unexpected answer with status {status}")]
    Server { status: u16 },
    /// The request never got an HTTP answer.
    #[error("the request never got an answer")]
    Network,
    /// A paid call failed before its request went out (the session, the token or the tool list): nothing started,
    /// so it can be sent again.
    #[error("the request was not sent")]
    NotSent,
    /// The operation was cancelled by its caller (Swift's `CancellationError`).
    #[error("cancelled")]
    Cancelled,
}

impl HiggsfieldError {
    /// `.validation(field: nil, message:)`.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            field: None,
            message: message.into(),
        }
    }
}

/// A generation as the server reports it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenerationStatus {
    Queued,
    InProgress,
    Completed {
        video_url: Url,
    },
    Failed {
        message: Option<String>,
    },
    /// Refused by content moderation, and not charged.
    Rejected(Moderation),
    Canceled,
}

/// An image generation as the server reports it: like `GenerationStatus`, but it finishes with several images.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageStatus {
    Queued,
    InProgress,
    /// Every https image URL of the answer, in its order; never empty.
    Completed {
        image_urls: Vec<Url>,
    },
    Failed {
        message: Option<String>,
    },
    Rejected(Moderation),
    Canceled,
}

/// Why moderation refused a generation, by the status the server reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Moderation {
    /// `nsfw`: explicit content.
    ExplicitContent,
    /// `ip_detected`: a recognizable real person, or a protected character or brand.
    ProtectedContent,
}

impl Moderation {
    /// The status the server reports.
    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::ExplicitContent => "nsfw",
            Self::ProtectedContent => "ip_detected",
        }
    }
}
