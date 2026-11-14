//! Why a job failed.

use serde::{Deserialize, Serialize};

/// Why a job failed. The app's own reasons are cases, so they are recognised without comparing text; a reason the
/// server or a provider gave is kept as its text. Engine steps return it to fail their job with it.
///
/// A message is stored as its plain text, the shape every stored reason had before the cases existed; the app's
/// own reasons as `{"code": …}`, which no text can be mistaken for. A code this version doesn't know (written by a
/// newer one) reads as `internalError`, so the job still loads.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = FailureReasonRepr))]
#[serde(into = "FailureReasonRepr", from = "FailureReasonRepr")]
pub enum FailureReason {
    /// The provider didn't finish the job within the generation deadline. It may still finish and charge for it,
    /// so the job is checked again rather than submitted again (`Job::may_still_finish`).
    #[error("deadline")]
    Deadline,
    /// A submit may have reached the server and was never answered, or a cancel cut it off.
    #[error("outcomeUnknown")]
    OutcomeUnknown,
    /// A submit without an idempotency key was cut off by a quit, crash or pause, so the server may have started it.
    #[error("submitCutOff")]
    SubmitCutOff,
    /// The disk filled up before the job was submitted; nothing was charged.
    #[error("notEnoughDiskSpace")]
    NotEnoughDiskSpace,
    /// The finished result expired on the server.
    #[error("outputUnavailable")]
    OutputUnavailable,
    /// An input reused from an earlier generation is gone from the server.
    #[error("inputUnavailable")]
    InputUnavailable,
    /// An input reused from an earlier generation doesn't point to an https address.
    #[error("insecureInput")]
    InsecureInput,
    /// Refused by content moderation before the provider accepted the job.
    #[error("contentModeration")]
    ContentModeration,
    /// The provider reported the generation as failed without saying why.
    #[error("generationFailed")]
    GenerationFailed,
    /// The provider canceled a job that could no longer be canceled from the app.
    #[error("canceledOnServer")]
    CanceledOnServer,
    /// A bug in the app, or an error that has no description to show.
    #[error("internalError")]
    InternalError,
    /// Text from the server or a provider, shown as it came.
    #[error("{0}")]
    Message(String),
}

impl FailureReason {
    /// Every reason but `Message`.
    pub const CODED: [Self; 11] = [
        Self::Deadline,
        Self::OutcomeUnknown,
        Self::SubmitCutOff,
        Self::NotEnoughDiskSpace,
        Self::OutputUnavailable,
        Self::InputUnavailable,
        Self::InsecureInput,
        Self::ContentModeration,
        Self::GenerationFailed,
        Self::CanceledOnServer,
        Self::InternalError,
    ];

    /// The stored name of a reason of the app's own; `None` for a message. Never change one: stored jobs hold it.
    pub fn code(&self) -> Option<&'static str> {
        Some(match self {
            Self::Deadline => "deadline",
            Self::OutcomeUnknown => "outcomeUnknown",
            Self::SubmitCutOff => "submitCutOff",
            Self::NotEnoughDiskSpace => "notEnoughDiskSpace",
            Self::OutputUnavailable => "outputUnavailable",
            Self::InputUnavailable => "inputUnavailable",
            Self::InsecureInput => "insecureInput",
            Self::ContentModeration => "contentModeration",
            Self::GenerationFailed => "generationFailed",
            Self::CanceledOnServer => "canceledOnServer",
            Self::InternalError => "internalError",
            Self::Message(_) => return None,
        })
    }
}

/// The stored shape of a `FailureReason`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged, rename = "FailureReason")]
pub enum FailureReasonRepr {
    /// A message, as its text.
    Message(String),
    /// One of the app's own reasons.
    Coded {
        /// `FailureReason::code`.
        code: String,
    },
}

impl From<FailureReason> for FailureReasonRepr {
    fn from(reason: FailureReason) -> Self {
        match reason {
            FailureReason::Message(text) => Self::Message(text),
            coded => Self::Coded {
                code: coded.code().unwrap_or("internalError").to_owned(),
            },
        }
    }
}

impl From<FailureReasonRepr> for FailureReason {
    fn from(repr: FailureReasonRepr) -> Self {
        match repr {
            FailureReasonRepr::Message(text) => Self::Message(text),
            FailureReasonRepr::Coded { code } => Self::CODED
                .into_iter()
                .find(|reason| reason.code() == Some(code.as_str()))
                .unwrap_or(Self::InternalError),
        }
    }
}
