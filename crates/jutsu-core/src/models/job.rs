//! One generation, from the moment it is queued until it is in the library.

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::job::{FailureReason, JobState};
use crate::models::{AgentOrigin, Cost, JobSpec, PixelSize};
use crate::provider::{ProviderKind, Rejection, RemoteInputs, RemoteJob, ViralityScore};
use crate::support::Seconds;
use crate::support::codable::Empty;

/// One generation, from the moment it is queued until it is in the library.
/// The engine owns its lifecycle; a `JobRepository` persists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// Its identity.
    pub id: Uuid,
    /// What it generates.
    pub spec: JobSpec,
    /// The connection kind that runs it.
    pub provider_kind: ProviderKind,
    /// `GenerationProvider::owner` of the connection that queued the job; only that connection may act on it.
    /// `None` in jobs stored before this existed, which the engine assigns as it loads them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The library item this job was made from ("Render final", "Edit & Re-run"); it may have been deleted
    /// since. `None` for a job started from scratch and in jobs stored before this existed.
    #[serde(default, rename = "parentID", skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// Shared by the jobs of one batch (one range × several styles or character sets), which the queue and the
    /// library show as a group. `None` for a job queued on its own and in jobs stored before batches existed.
    #[serde(default, rename = "batchID", skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Uuid>,
    /// Shared by the variants one submit started.
    #[serde(default, rename = "variantGroupID", skip_serializing_if = "Option::is_none")]
    pub variant_group_id: Option<Uuid>,
    /// Where it is.
    pub state: JobState,
    /// 0…1 within the current state; only preparing, uploading, processing and downloading report it.
    pub progress: f64,
    /// Sent with every submit of one attempt: network retries, a lost submit response and resume after a crash
    /// reuse it, so they can't charge twice. A user retry of an uncharged `failed` or `canceled` job gets a new
    /// key, unless a submit of it may have reached the server: the old key then lets the provider return that
    /// request instead of charging for another.
    pub idempotency_key: Uuid,
    /// The preliminary price, from the list price or the provider's estimate when the job was queued.
    pub cost: Cost,
    /// What the provider quoted for the uploaded inputs right before submit; `None` until then, or when it
    /// can't quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_price: Option<Cost>,
    /// A price the user agreed to over the daily cap: the estimate when queued, or a server price quoted later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_price: Option<Cost>,
    /// The uploaded inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<RemoteInputs>,
    /// The submitted job as the provider knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteJob>,
    /// Saved right before a submit is sent, cleared once it is answered. Still set after a crash means the server
    /// may have the job; `None` in jobs stored before this existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_attempt: Option<SubmitAttempt>,
    /// The downloaded result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<Url>,
    /// The downloaded result's own length and frame size, read from the file. `None` before the download, when
    /// the file can't be read, and in jobs stored before this existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_media: Option<OutputMedia>,
    /// The output folder the running download started in. Its partial file and the result stay there even if
    /// the output folder changes meanwhile, so the download resumes. `None` while no download runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_folder: Option<Url>,
    /// When the API accepted the submit; `None` before that and in jobs stored before this existed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub submitted_at: Option<OffsetDateTime>,
    /// When the API first reported the job in progress; `None` if no poll saw that state.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub processing_at: Option<OffsetDateTime>,
    /// When the user moved the job ahead of the other jobs waiting for a slot (Run next).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub promoted_at: Option<OffsetDateTime>,
    /// When the API finished the generation, which is when it charges. Set even if the download is later canceled
    /// or fails; `None` in jobs stored before this existed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub charged_at: Option<OffsetDateTime>,
    /// When the deadline failed the job while the provider could still finish, and charge for, its generation;
    /// `None` otherwise.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub timed_out_at: Option<OffsetDateTime>,
    /// When the user last asked to check a job past its deadline again; its deadline counts from here then.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub checked_again_at: Option<OffsetDateTime>,
    /// When it was queued.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// When the last Retry began a new attempt; `None` before one and in jobs stored before this existed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub retried_at: Option<OffsetDateTime>,
    /// Why moderation refused the job; `None` for any other job, and in rejected jobs stored before this existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<Rejection>,
    /// When it ended.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub finished_at: Option<OffsetDateTime>,
    /// When a generation made outside this app was added to the library; `None` for jobs the app ran.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub imported_at: Option<OffsetDateTime>,
    /// The step that keeps failing and waits to run again; `None` once a step gets through, and in jobs stored
    /// before this existed. Kept on a job paused for `KeptFailing`, for what it failed with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrying: Option<RetryStatus>,
    /// The result's virality prediction, kept so it isn't paid for again; `None` until scored, and in jobs stored
    /// before this existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virality: Option<ViralityScore>,
    /// Variants its submit asked for (`JobSpec::variants`) that the answer started no job for; `None` when it
    /// started them all, and in jobs stored before variants existed. Set on the job that sent the submit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_variants: Option<i64>,
    /// The agent that queued the job, and what it may spend; `None` for a job the person queued, and in jobs stored
    /// before agents existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentOrigin>,
}

impl Job {
    /// A new job, preparing, with no progress.
    pub fn new(
        id: Uuid,
        spec: JobSpec,
        provider_kind: ProviderKind,
        cost: Cost,
        idempotency_key: Uuid,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            spec,
            provider_kind,
            owner: None,
            parent_id: None,
            batch_id: None,
            variant_group_id: None,
            state: JobState::Preparing,
            progress: 0.0,
            idempotency_key,
            cost,
            server_price: None,
            approved_price: None,
            inputs: None,
            remote: None,
            submit_attempt: None,
            output_file: None,
            output_media: None,
            download_folder: None,
            submitted_at: None,
            processing_at: None,
            promoted_at: None,
            charged_at: None,
            timed_out_at: None,
            checked_again_at: None,
            created_at,
            retried_at: None,
            rejection: None,
            finished_at: None,
            imported_at: None,
            retrying: None,
            virality: None,
            missing_variants: None,
            agent: None,
        }
    }

    /// The API finished the generation and charged for it, so its result can be fetched again.
    pub fn has_paid_output(&self) -> bool {
        self.charged_at.is_some() && self.remote.is_some()
    }

    /// Failed at the deadline with its request still known: the provider may yet finish it and charge for it,
    /// so it is checked again rather than submitted again.
    pub fn may_still_finish(&self) -> bool {
        self.state
            == JobState::Failed {
                reason: FailureReason::Deadline,
            }
            && self.remote.is_some()
            && self.charged_at.is_none()
    }

    /// Failed without knowing whether the provider charged for it: past the deadline (`may_still_finish`), or with
    /// a submit that may have reached the server and was never answered (`submit_attempt` kept).
    pub fn may_have_been_charged(&self) -> bool {
        if !matches!(self.state, JobState::Failed { .. }) || self.charged_at.is_some() {
            return false;
        }
        self.may_still_finish() || self.submit_attempt.is_some()
    }

    /// The API charges when it finishes a generation, so a job whose download then failed or was canceled
    /// is charged too. Jobs stored before `charged_at` existed are charged when they are done.
    pub fn is_charged(&self) -> bool {
        self.state == JobState::Done || self.charged_at.is_some()
    }

    /// How long the result runs: the file's own length once read, else what the job asked for
    /// (`JobSpec::duration`). A provider's job may not run as long as it asked (a Seedance Edit asked 5 s and ran
    /// 3.71 s).
    pub fn output_duration(&self) -> Seconds {
        self.output_media
            .as_ref()
            .map_or_else(|| self.spec.duration(), |media| media.duration)
    }

    /// The result's frame size: the file's own once read, else the source's; `None` while neither is known.
    pub fn output_pixel_size(&self) -> Option<PixelSize> {
        if let Some(media) = &self.output_media {
            return Some(media.pixel_size);
        }
        self.spec
            .source
            .as_ref()
            .map(|source| source.pixel_size)
            .filter(PixelSize::is_valid)
    }

    /// The best known price: the server's once it quoted one, the preliminary one before.
    pub fn price(&self) -> &Cost {
        self.server_price.as_ref().unwrap_or(&self.cost)
    }

    /// A job whose price wasn't known when it was queued and that no answer has priced since.
    pub fn price_is_unknown(&self) -> bool {
        self.price().is_unknown_price()
    }

    /// When the current attempt began: the last Retry, or when the job was queued.
    pub fn attempt_started_at(&self) -> OffsetDateTime {
        self.retried_at.unwrap_or(self.created_at)
    }

    /// From queueing, or the last Retry, until the API started generating; `None` until it started.
    pub fn queue_wait(&self) -> Option<Duration> {
        self.processing_at
            .map(|processing| processing - self.attempt_started_at())
    }

    /// From the start of the generation until the API finished it; `None` unless both are known.
    pub fn processing_time(&self) -> Option<Duration> {
        Some(self.charged_at? - self.processing_at?)
    }
}

/// What a downloaded result's file holds, read from the file itself rather than from what the job asked for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OutputMedia {
    /// In seconds.
    pub duration: Seconds,
    /// Upright, as the file shows (its rotation applied).
    pub pixel_size: PixelSize,
}

/// What the last attempt of a retried step failed with. Coded as Swift codes the enum, `{"server":{"status":503}}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = RetryCauseRepr))]
#[serde(into = "RetryCauseRepr", from = "RetryCauseRepr")]
pub enum RetryCause {
    /// The network failed.
    Network,
    /// The server answered with this status.
    Server {
        /// The HTTP status.
        status: i64,
    },
    /// The model isn't available.
    ModelUnavailable,
    /// All of the account's slots are busy.
    ConcurrencyLimit,
    /// Out of funds after submit, while the job is polled or downloaded.
    InsufficientFunds,
}

/// The stored shape of a `RetryCause`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "RetryCause")]
#[allow(missing_docs, reason = "each case mirrors `RetryCause`")]
pub enum RetryCauseRepr {
    Network(Empty),
    Server { status: i64 },
    ModelUnavailable(Empty),
    ConcurrencyLimit(Empty),
    InsufficientFunds(Empty),
}

impl From<RetryCause> for RetryCauseRepr {
    fn from(cause: RetryCause) -> Self {
        match cause {
            RetryCause::Network => Self::Network(Empty {}),
            RetryCause::Server { status } => Self::Server { status },
            RetryCause::ModelUnavailable => Self::ModelUnavailable(Empty {}),
            RetryCause::ConcurrencyLimit => Self::ConcurrencyLimit(Empty {}),
            RetryCause::InsufficientFunds => Self::InsufficientFunds(Empty {}),
        }
    }
}

impl From<RetryCauseRepr> for RetryCause {
    fn from(repr: RetryCauseRepr) -> Self {
        match repr {
            RetryCauseRepr::Network(_) => Self::Network,
            RetryCauseRepr::Server { status } => Self::Server { status },
            RetryCauseRepr::ModelUnavailable(_) => Self::ModelUnavailable,
            RetryCauseRepr::ConcurrencyLimit(_) => Self::ConcurrencyLimit,
            RetryCauseRepr::InsufficientFunds(_) => Self::InsufficientFunds,
        }
    }
}

/// A step of a job that failed for a reason waiting can fix, and is tried again with backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RetryStatus {
    /// Failed attempts in a row, 1 after the first.
    pub failures: i64,
    /// When the first of them failed.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub since: OffsetDateTime,
    /// When the step runs again.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub next_attempt_at: OffsetDateTime,
    /// What the last attempt failed with.
    pub cause: RetryCause,
}

/// A submit that was sent and not yet answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SubmitAttempt {
    /// The provider sent the job's idempotency key, so sending the submit again can't charge twice.
    pub is_idempotent: bool,
}
