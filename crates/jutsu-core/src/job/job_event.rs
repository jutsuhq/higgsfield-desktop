//! What happens to a job.

use crate::job::{FailureReason, PauseReason};

/// Something that happened to a job: a pipeline step finished, the API answered, or the user acted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JobEvent {
    /// The inputs are prepared.
    Prepared,
    /// The inputs are uploaded.
    Uploaded,
    /// The API accepted the job and returned a request ID.
    Submitted,
    /// Submit was refused because all of the account's slots are busy.
    HitConcurrencyLimit,
    /// The API started the job.
    Started,
    /// The API finished the job.
    Completed,
    /// The result is downloaded.
    Downloaded,
    /// The job failed.
    Failed {
        /// Why.
        reason: FailureReason,
    },
    /// Refused by content moderation.
    Rejected,
    /// Paused, for this reason.
    Paused(PauseReason),
    /// The user cancels it.
    Cancel,
    /// The user resumes it.
    Resume,
    /// The user retries it.
    Retry,
    /// The user asks for the result of a generation that already finished, without generating again.
    RetryDownload,
    /// The user asks to poll a generation that failed at the deadline again; the provider may still finish it.
    CheckAgain,
}
