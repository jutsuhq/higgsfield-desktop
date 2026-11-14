//! Virality predictions of finished results.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

use crate::models::Cost;
use crate::ports::PortError;

/// What a virality prediction said about one result. Stored on the job (`Job::virality`), so a result is never
/// scored again without the user asking. Scores stored before the live answer was known carry other keys, which are
/// ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ViralityScore {
    /// The overall score, 0–100; `None` when the provider only linked a report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    /// 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viral_potential: Option<i64>,
    /// How strongly the first 3 seconds hold attention, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<i64>,
    /// 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engagement: Option<i64>,
    /// How well attention holds over the whole video, 0–100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sustain: Option<i64>,
    /// When the video reads strongest, in seconds from its start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_second: Option<f64>,
    /// One reading per frame, on the provider's own scale; empty or `None` when it gave none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_frame: Option<Vec<f64>>,
    /// What the provider says the scores are and aren't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclaimer: Option<String>,
    /// The provider's dashboard of the report.
    #[serde(default, rename = "reportURL", skip_serializing_if = "Option::is_none")]
    pub report_url: Option<Url>,
    /// When it was scored.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub scored_at: OffsetDateTime,
}

/// The answer of one scoring call.
#[derive(Debug, Clone, PartialEq)]
pub struct ViralityOutcome {
    /// `None` when the answer couldn't be read; the call may still have been charged.
    pub score: Option<ViralityScore>,
    /// What the provider says it charged; `None` when it doesn't say.
    pub charged: Option<Cost>,
}

/// A provider that predicts how well a finished result of its own account will do: hook, sustain and engagement.
/// Named by the result's job ID, so nothing is uploaded.
#[async_trait]
pub trait ViralityScoring: Send + Sync {
    /// The price of scoring one result; `None` when the provider gives no quote.
    async fn virality_estimate(&self, job_id: &str) -> Result<Option<Cost>, PortError>;
    /// Scores one result, waiting until the provider has. Fails with `ProviderError::OutcomeUnknown` when a paid
    /// call may have reached the provider without an answer, any other `ProviderError` when it was refused or
    /// failed.
    async fn score_virality(&self, job_id: &str) -> Result<ViralityOutcome, PortError>;
}
