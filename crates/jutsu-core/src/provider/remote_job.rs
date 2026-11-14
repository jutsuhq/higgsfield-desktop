//! A submitted job as the provider knows it.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::Cost;
use crate::provider::RemoteStatus;

/// A submitted job as the provider knows it. Only `requestID`, `correlationID`, `statusURL`, `cancelURL` and
/// `isNativeDraft` are stored; the rest is what the submit answer said, which the engine moves elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoteJob {
    /// The provider's ID of the job.
    #[serde(rename = "requestID")]
    pub request_id: String,
    /// Server-side trace ID, kept for support requests; `None` when the provider doesn't return one.
    #[serde(default, rename = "correlationID", skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Where the provider said to poll; `None` when it didn't say. Check the origin before use.
    #[serde(default, rename = "statusURL", skip_serializing_if = "Option::is_none")]
    pub status_url: Option<Url>,
    /// Where the provider said to cancel; `None` when it didn't say. Check the origin before use.
    #[serde(default, rename = "cancelURL", skip_serializing_if = "Option::is_none")]
    pub cancel_url: Option<Url>,
    /// What the submit answer said about the job, when it said anything. Not stored.
    #[serde(skip)]
    pub initial_status: Option<RemoteStatus>,
    /// The submit was sent to be paid from the plan's unlimited allowance (`JobSpec::uses_unlimited`), and the
    /// answer didn't say it cost credits. Not stored: the engine keeps it in the job's spec.
    #[serde(skip)]
    pub paid_with_unlimited: bool,
    /// What the submit answer says the job costs, when it says so; `None` otherwise. Not stored: the engine keeps
    /// it as the job's server price.
    #[serde(skip)]
    pub charged: Option<Cost>,
    /// The other jobs the same submit started, one per further variant (`JobSpec::variants`), in the answer's
    /// order; each of them is paid for. Not stored: the engine makes each one a job of its own.
    #[serde(skip)]
    pub variants: Vec<RemoteJob>,
    /// The submit asked for Seedance's native draft on the account (`draft: true`), so this job, once completed,
    /// can be finalized at 1080p by its ID within seven days (`JobSpec::draft_job_id`). Stored; `false` in jobs
    /// stored before it existed.
    #[serde(default, rename = "isNativeDraft")]
    pub is_native_draft: bool,
}

impl RemoteJob {
    /// The job of ID `request_id`, with nothing else known.
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            correlation_id: None,
            status_url: None,
            cancel_url: None,
            initial_status: None,
            paid_with_unlimited: false,
            charged: None,
            variants: Vec::new(),
            is_native_draft: false,
        }
    }
}
