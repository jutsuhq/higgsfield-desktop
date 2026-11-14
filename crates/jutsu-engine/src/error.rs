//! What the engine's calls fail with, and the errors its steps pass along.

use std::io;

use jutsu_core::{BudgetError, Cost, FailureReason, JobEvent, JobState, PortError, ProviderError, SpendCap};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::downloader::{DownloadError, DownloadRefusal};

/// Why a call of the engine failed. The text of each case is the macOS app's wording.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// The job isn't known.
    #[error("This job is no longer in the queue.")]
    UnknownJob(Uuid),
    /// No connection, or the connection belongs to another provider or account than the job's.
    #[error("Connect the Higgsfield account or API key this job was made with, then try again.")]
    NotConnected,
    /// The computer is offline.
    #[error("Your computer is offline. Try again once it is connected.")]
    Offline,
    /// Once the API starts processing a job it can't be canceled.
    #[error("Higgsfield is already making this job, so it can't be canceled.")]
    NotCancellable(JobState),
    /// A submit sent with the idempotency key is waiting for its answer; the job can be canceled once queued.
    #[error("Higgsfield may already be starting this job. Cancel it once it shows as queued.")]
    SubmitInProgress,
    /// The job would take today's spend over the cap, and the user hasn't agreed to that.
    #[error(
        "Jobs queued meanwhile took today's spend close to your cap. Try again to confirm going over it."
    )]
    OverCap(OverCap),
    /// Only a failed or canceled job is retried.
    #[error("This job can't be retried.")]
    NotRetryable(JobState),
    /// Only a paused job is resumed.
    #[error("This job isn't paused.")]
    NotResumable(JobState),
    /// Run next only reorders jobs still waiting for a slot.
    #[error("Only a job waiting for its turn can run next.")]
    NotWaiting(JobState),
    /// The download URL answered with a client error (4xx); the engine decides whether waiting can help.
    #[error("The server refused to send the file (HTTP {status}).")]
    DownloadFailed {
        /// The HTTP status.
        status: u16,
    },
    /// The download was stopped; retrying won't help.
    #[error("{0}")]
    DownloadRefused(DownloadRefusal),
    /// A submitted job was stored without its request ID.
    #[error("This job was saved without its Higgsfield request, so its result can't be found.")]
    MissingRequestId,
    /// A bug in the engine: `event` can't happen in `from`.
    #[error("Internal error.")]
    InvalidTransition {
        /// The state.
        from: JobState,
        /// The event.
        event: JobEvent,
    },
    /// Today's spending couldn't be read.
    #[error(transparent)]
    Budget(#[from] BudgetError),
    /// A port (the store, the provider, the ledger) failed.
    #[error(transparent)]
    Port(#[from] PortError),
    /// A file couldn't be read or written.
    #[error("{message}")]
    Io {
        /// What kind of failure.
        kind: io::ErrorKind,
        /// Its description.
        message: String,
    },
    /// The work was cancelled.
    #[error("cancelled")]
    Cancelled,
}

/// What the person's daily cap counted when it refused a job.
#[derive(Debug, Clone, PartialEq)]
pub struct OverCap {
    /// The cap.
    pub cap: SpendCap,
    /// Every other job the cap counts today, charged or still queued, in the cap's unit.
    pub committed_today: Decimal,
    /// What the job was counted at.
    pub price: Cost,
}

/// Why a step of a job didn't get through; `recover` decides what that does to the job.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StepError {
    Provider(ProviderError),
    Failure(FailureReason),
    Engine(EngineError),
    Cancelled,
}

pub(crate) type Step<T = ()> = Result<T, StepError>;

impl From<ProviderError> for StepError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl From<FailureReason> for StepError {
    fn from(reason: FailureReason) -> Self {
        Self::Failure(reason)
    }
}

impl From<EngineError> for StepError {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::Cancelled => Self::Cancelled,
            EngineError::Port(port) => port.into(),
            other => Self::Engine(other),
        }
    }
}

impl From<PortError> for StepError {
    fn from(error: PortError) -> Self {
        match error {
            PortError::Provider(provider) => Self::Provider(provider),
            PortError::Failure(reason) => Self::Failure(reason),
            PortError::Cancelled => Self::Cancelled,
            other => Self::Engine(EngineError::Port(other)),
        }
    }
}

impl From<BudgetError> for StepError {
    fn from(error: BudgetError) -> Self {
        Self::Engine(EngineError::Budget(error))
    }
}

impl From<io::Error> for StepError {
    fn from(error: io::Error) -> Self {
        Self::Engine(EngineError::io(&error))
    }
}

impl From<crate::cancel::Cancelled> for StepError {
    fn from(_: crate::cancel::Cancelled) -> Self {
        Self::Cancelled
    }
}

impl From<DownloadError> for StepError {
    fn from(error: DownloadError) -> Self {
        match error {
            DownloadError::Provider(provider) => Self::Provider(provider),
            DownloadError::Failed { status } => Self::Engine(EngineError::DownloadFailed { status }),
            DownloadError::Refused(refusal) => Self::Engine(EngineError::DownloadRefused(refusal)),
            DownloadError::Io { kind, message } => Self::Engine(EngineError::Io { kind, message }),
            DownloadError::NotAccepted(error) => {
                Self::Engine(EngineError::Port(PortError::Other(error.to_string())))
            }
            DownloadError::Cancelled => Self::Cancelled,
        }
    }
}

impl From<StepError> for EngineError {
    fn from(error: StepError) -> Self {
        match error {
            StepError::Provider(provider) => Self::Port(PortError::Provider(provider)),
            StepError::Failure(reason) => Self::Port(PortError::Failure(reason)),
            StepError::Engine(engine) => engine,
            StepError::Cancelled => Self::Cancelled,
        }
    }
}

impl EngineError {
    /// An I/O failure, kept by kind so a full disk is recognised.
    pub(crate) fn io(error: &io::Error) -> Self {
        Self::Io {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl StepError {
    /// The disk or the user's quota is full (`ENOSPC`, `EDQUOT`), or a step reported that it is.
    pub(crate) fn is_out_of_space(&self) -> bool {
        match self {
            Self::Engine(EngineError::Io { kind, .. }) => is_out_of_space(*kind),
            Self::Failure(FailureReason::NotEnoughDiskSpace) => true,
            _ => false,
        }
    }

    /// The reason a job fails with for an error no step handles: out of disk space before submit, else the error's
    /// own description. An error without one is a bug to the user.
    pub(crate) fn failure_reason(&self) -> FailureReason {
        if self.is_out_of_space() {
            return FailureReason::NotEnoughDiskSpace;
        }
        let description = match self {
            Self::Provider(provider) => provider.to_string(),
            Self::Failure(reason) => return reason.clone(),
            Self::Engine(engine) => engine.to_string(),
            Self::Cancelled => String::new(),
        };
        if description.is_empty() {
            FailureReason::InternalError
        } else {
            FailureReason::Message(description)
        }
    }
}

/// `ENOSPC` or `EDQUOT`.
pub(crate) fn is_out_of_space(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app runs on more than macOS, so the message names the computer.
    #[test]
    fn offline_names_the_computer() {
        assert_eq!(
            EngineError::Offline.to_string(),
            "Your computer is offline. Try again once it is connected."
        );
    }
}
