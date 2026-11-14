//! Where a job is in its lifecycle, and the transitions between states.

use serde::{Deserialize, Serialize};

use crate::job::{FailureReason, JobEvent};
use crate::support::codable::{Empty, Unlabeled};

/// Where a job is. Coded as Swift codes the enum: `{"queued":{}}`, `{"failed":{"reason":…}}`,
/// `{"paused":{"_0":"byUser"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = JobStateRepr))]
#[serde(into = "JobStateRepr", from = "JobStateRepr")]
pub enum JobState {
    /// Preparing its inputs.
    Preparing,
    /// Uploading its inputs.
    Uploading,
    /// Waiting for one of the account's slots.
    WaitingForSlot,
    /// Submitted; the API hasn't started it yet.
    Queued,
    /// The API is generating it.
    Processing,
    /// Downloading the result.
    Downloading,
    /// Finished.
    Done,
    /// Failed.
    Failed {
        /// Why.
        reason: FailureReason,
    },
    /// Refused by content moderation (`nsfw`).
    Rejected,
    /// Canceled.
    Canceled,
    /// Paused, for this reason.
    Paused(PauseReason),
}

/// Why a job is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    /// The account can't pay for it.
    InsufficientFunds,
    /// The credential was rejected or removed; the user has to sign in again.
    ConnectionNeedsAttention,
    /// The provider's price for the uploaded inputs takes today's spend over the cap, and the preliminary
    /// price didn't; resuming agrees to that price.
    PriceAboveCap,
    /// The finished, already charged result couldn't be written for lack of disk space. The partial file and the
    /// request stay, so resuming continues the download.
    NotEnoughDiskSpace,
    /// A step before submit kept failing for a reason that waiting usually fixes (network, server, model
    /// unavailable), for longer than the engine retries on its own; `Job::retrying` says what with. Nothing is
    /// charged yet; resuming tries again.
    KeptFailing,
    /// Pause all; only the user lifts it.
    ByUser,
    /// An agent's job the spending guard stopped before its submit (`AgentOrigin::hold` says why): over the agent's
    /// `max_cost`, over today's agent budget, or at an unknown price. Nothing is charged; only the person resumes
    /// it, which agrees to the price, and never an agent or Resume all.
    AgentLimit,
}

impl JobState {
    /// Nothing more will happen without the user (retry or a new job).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed { .. } | Self::Rejected | Self::Canceled
        )
    }

    /// Still in the queue, paused jobs included.
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// Not yet started by the API, which can't cancel a job once it is processing, or already
    /// generated and only downloading, which is local.
    pub fn is_cancellable(&self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Uploading
                | Self::WaitingForSlot
                | Self::Queued
                | Self::Paused(_)
                | Self::Downloading
        )
    }

    /// Not yet submitted, so a user pause keeps it from taking a slot. A connection pause lifts by itself
    /// once the connection is back, so a user pause takes it over; a funds pause already waits for the user.
    pub fn is_pausable_by_user(&self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Uploading
                | Self::WaitingForSlot
                | Self::Paused(PauseReason::ConnectionNeedsAttention)
        )
    }

    /// The job's lifecycle, in the order of the Swift original. Returns `None` when `event` can't happen in `state`; callers treat that as a bug.
    #[allow(clippy::match_same_arms, reason = "kept in the order of the Swift original")]
    pub fn transition(state: &Self, event: &JobEvent) -> Option<Self> {
        use JobEvent as E;
        use JobState as S;
        match (state, event) {
            (S::Preparing, E::Prepared) => Some(S::Uploading),
            (S::Uploading, E::Uploaded) => Some(S::WaitingForSlot),
            (S::WaitingForSlot, E::Submitted) => Some(S::Queued),
            (S::WaitingForSlot, E::HitConcurrencyLimit) => Some(S::WaitingForSlot),
            (S::Queued, E::Started) => Some(S::Processing),
            // A poll can miss `in_progress` entirely.
            (S::Queued | S::Processing, E::Completed) => Some(S::Downloading),
            (S::Downloading, E::Downloaded) => Some(S::Done),
            (S::Queued | S::Processing, E::Rejected) => Some(S::Rejected),
            (_, E::Paused(PauseReason::ByUser)) if state.is_pausable_by_user() => {
                Some(S::Paused(PauseReason::ByUser))
            }
            // Only before submit: a job the API has accepted keeps its state, the engine just stops polling.
            (S::Preparing | S::Uploading | S::WaitingForSlot, E::Paused(reason)) => {
                (*reason != PauseReason::NotEnoughDiskSpace).then_some(S::Paused(*reason))
            }
            // A full disk only pauses the download of a paid result, and resuming continues it.
            (S::Downloading, E::Paused(PauseReason::NotEnoughDiskSpace)) => {
                Some(S::Paused(PauseReason::NotEnoughDiskSpace))
            }
            (S::Paused(PauseReason::NotEnoughDiskSpace), E::Resume) => Some(S::Downloading),
            // Restart the pipeline; prepared files and uploads are cached by content hash, so this is cheap.
            (S::Paused(_), E::Resume) | (S::Failed { .. } | S::Canceled, E::Retry) => Some(S::Preparing),
            (S::Failed { .. } | S::Canceled, E::RetryDownload) => Some(S::Downloading),
            (S::Failed { .. }, E::CheckAgain) => Some(S::Queued),
            (_, E::Cancel) if state.is_cancellable() => Some(S::Canceled),
            (S::Paused(_), E::Failed { .. }) => None,
            (_, E::Failed { reason }) if state.is_active() => Some(S::Failed {
                reason: reason.clone(),
            }),
            _ => None,
        }
    }
}

/// The stored shape of a `JobState`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "JobState")]
#[allow(missing_docs, reason = "each case mirrors `JobState`")]
pub enum JobStateRepr {
    Preparing(Empty),
    Uploading(Empty),
    WaitingForSlot(Empty),
    Queued(Empty),
    Processing(Empty),
    Downloading(Empty),
    Done(Empty),
    Failed { reason: FailureReason },
    Rejected(Empty),
    Canceled(Empty),
    Paused(Unlabeled<PauseReason>),
}

impl From<JobState> for JobStateRepr {
    fn from(state: JobState) -> Self {
        let empty = Empty {};
        match state {
            JobState::Preparing => Self::Preparing(empty),
            JobState::Uploading => Self::Uploading(empty),
            JobState::WaitingForSlot => Self::WaitingForSlot(empty),
            JobState::Queued => Self::Queued(empty),
            JobState::Processing => Self::Processing(empty),
            JobState::Downloading => Self::Downloading(empty),
            JobState::Done => Self::Done(empty),
            JobState::Failed { reason } => Self::Failed { reason },
            JobState::Rejected => Self::Rejected(empty),
            JobState::Canceled => Self::Canceled(empty),
            JobState::Paused(reason) => Self::Paused(Unlabeled { value: reason }),
        }
    }
}

impl From<JobStateRepr> for JobState {
    fn from(repr: JobStateRepr) -> Self {
        match repr {
            JobStateRepr::Preparing(_) => Self::Preparing,
            JobStateRepr::Uploading(_) => Self::Uploading,
            JobStateRepr::WaitingForSlot(_) => Self::WaitingForSlot,
            JobStateRepr::Queued(_) => Self::Queued,
            JobStateRepr::Processing(_) => Self::Processing,
            JobStateRepr::Downloading(_) => Self::Downloading,
            JobStateRepr::Done(_) => Self::Done,
            JobStateRepr::Failed { reason } => Self::Failed { reason },
            JobStateRepr::Rejected(_) => Self::Rejected,
            JobStateRepr::Canceled(_) => Self::Canceled,
            JobStateRepr::Paused(Unlabeled { value }) => Self::Paused(value),
        }
    }
}
