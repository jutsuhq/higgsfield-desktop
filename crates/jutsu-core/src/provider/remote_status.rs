//! Where submitted jobs are.

use std::collections::BTreeMap;

use time::Duration;
use url::Url;

use crate::support::string_enum;

/// Where a submitted job is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RemoteStatus {
    /// Not started.
    Queued,
    /// Generating.
    InProgress,
    /// Finished.
    Completed {
        /// The result.
        video_url: Url,
    },
    /// Failed, with the provider's message when it gave one.
    Failed {
        /// The message.
        message: Option<String>,
    },
    /// Refused by content moderation, and not charged.
    Rejected(Rejection),
    /// Canceled.
    Canceled,
}

/// The statuses of several submitted jobs from one call (`GenerationProvider::statuses`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStatus {
    /// By `RemoteJob::request_id`. A job missing here is asked for on its own.
    pub statuses: BTreeMap<String, RemoteStatus>,
    /// When the provider asks to be polled again; `None` when it doesn't say.
    pub poll_after: Option<Duration>,
}

string_enum! {
    /// Why content moderation refused a generation.
    pub enum Rejection {
        /// Explicit content.
        ExplicitContent = "explicitContent",
        /// A recognizable real person, or a protected character or brand.
        ProtectedContent = "protectedContent",
    }
}
