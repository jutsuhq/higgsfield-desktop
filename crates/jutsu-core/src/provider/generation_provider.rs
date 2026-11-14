//! A way to run jobs: the REST API with a key, or a Higgsfield account over MCP.

use async_trait::async_trait;
use url::Url;
use uuid::Uuid;

use crate::models::{Cost, Job, JobSpec, MediaLinkKind, StylePreset};
use crate::ports::PortError;
use crate::provider::{
    Balance, BatchStatus, Capabilities, PreparedFile, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus,
    ServerMedia,
};
use crate::support::string_enum;

string_enum! {
    /// How the connection reaches Higgsfield.
    pub enum ProviderKind {
        /// The REST API with a key, paying in dollars.
        ApiKey = "apiKey",
        /// A Higgsfield account over MCP, paying in credits.
        Account = "account",
    }
}

/// A way to run Genjutsu jobs: the REST API with a key, or a Higgsfield account over MCP. The engine and UI only
/// talk to the provider of the active connection.
#[async_trait]
pub trait GenerationProvider: Send + Sync {
    /// The connection kind.
    fn kind(&self) -> ProviderKind;
    /// Who pays: the key ID, or the signed-in account. Stable across launches and token refreshes; `None` when the
    /// account can't be identified.
    fn owner(&self) -> Option<String>;
    /// What the connection can run.
    async fn capabilities(&self) -> Result<Capabilities, PortError>;
    /// The restyle presets.
    async fn presets(&self) -> Result<Vec<StylePreset>, PortError>;
    /// The preliminary price, before anything is uploaded.
    async fn estimate(&self, spec: &JobSpec) -> Result<Cost, PortError>;
    /// The provider's own price for the exact request `submit` would send; `None` when it can't quote one.
    async fn server_estimate(
        &self,
        _spec: &JobSpec,
        _inputs: &RemoteInputs,
    ) -> Result<Option<Cost>, PortError> {
        Ok(None)
    }
    /// What the provider charged for `job`, which it finished without a price known beforehand
    /// (`Job::price_is_unknown`); `None` when it can't tell which charge was the job's.
    async fn charge(&self, _job: &Job) -> Result<Option<Cost>, PortError> {
        Ok(None)
    }
    /// Uploads a prepared file.
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError>;
    /// An input the provider can take straight from `origin`, the public `https:` link its file was downloaded
    /// from, so the file isn't uploaded; `None` when it has to be uploaded. `byte_count` is the file's size.
    async fn linked_asset(
        &self,
        _origin: &Url,
        _kind: MediaLinkKind,
        _byte_count: i64,
    ) -> Result<Option<RemoteAsset>, PortError> {
        Ok(None)
    }
    /// `media` as an input of this connection, so its file isn't uploaded; `None` when it has to be uploaded, as
    /// for media another account listed.
    async fn server_asset(&self, _media: &ServerMedia) -> Result<Option<RemoteAsset>, PortError> {
        Ok(None)
    }
    /// Reuse `idempotency_key` for every resubmit of the same attempt so it can't charge twice. A provider that
    /// can't send the key fails with `ProviderError::OutcomeUnknown` instead of any error the engine retries once
    /// the request may have reached the server.
    async fn submit(
        &self,
        spec: &JobSpec,
        inputs: &RemoteInputs,
        idempotency_key: Uuid,
    ) -> Result<RemoteJob, PortError>;
    /// Whether `submit` sends the idempotency key for `spec`, so it can be sent again after a crash.
    async fn submit_is_idempotent(&self, spec: &JobSpec) -> Result<bool, PortError>;
    /// Where a submitted job is.
    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError>;
    /// How many jobs `statuses` takes in one call. Below 2, the engine polls each job with `status` alone.
    fn status_batch_limit(&self) -> usize {
        0
    }
    /// The statuses of up to `status_batch_limit` jobs in one call; `None` when the provider can't give them this
    /// time, and the engine then asks `status` for each job.
    async fn statuses(&self, _jobs: &[RemoteJob]) -> Result<Option<BatchStatus>, PortError> {
        Ok(None)
    }
    /// Cancels a submitted job.
    async fn cancel(&self, job: &RemoteJob) -> Result<(), PortError>;
    /// `None` when the provider can't report a balance.
    async fn balance(&self) -> Result<Option<Balance>, PortError>;
    /// Whether `spec` is to be paid from the plan's unlimited allowance: the user opted in and the provider read
    /// that the account has it for the spec's model now. The engine asks when the job is queued.
    async fn pays_unlimited(&self, _spec: &JobSpec) -> bool {
        false
    }
    /// The connection `job` was queued with: the same kind and the same owner.
    fn owns(&self, job: &Job) -> bool {
        self.kind() == job.provider_kind && self.owner() == job.owner
    }
}
