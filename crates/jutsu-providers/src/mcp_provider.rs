//! The account connection: Genjutsu and the models in `account_model` through the tools of Higgsfield's MCP
//! server, paid with plan credits. Which modes and operations exist is decided by `tools/list`.

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use higgsfield::core::{HiggsfieldError, is_https};
use higgsfield::mcp::account::{
    Generation, GenerationType, HiggsfieldAccount, MediaRole, StyleRun, UnlimitedAllowance, VideoEdit,
    VideoEditOperation, VideoMedia, VideoRequest,
};
use jutsu_core::{
    AccountUploads, AccountVoice, Balance, BatchStatus, Capabilities, Cost, CostUnit, CreditHistory,
    CreditPage, GenerationHistory, GenerationLikes, GenerationProvider, HistoryCursor, HistoryPage, Job,
    JobSpec, MediaLinkKind, Mode, ModelConstraintTable, ModelConstraintsSource, MotionLibrary,
    MotionPresetPage, MotionShelf, PortError, PreparedFile, ProviderError, ProviderKind, RemoteAsset,
    RemoteInputs, RemoteJob, RemoteStatus, Resolution, ServerMedia, StylePreset, UploadKind, UploadPage,
    VideoModel,
};
use rust_decimal::Decimal;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::account_mapping::whole_number;
use crate::api_provider::file_path;
use crate::mapping::{not_available, port, provider, remote_status, server_credits, validation};
use crate::quote_cache::QuoteCache;
use crate::{account_mapping, account_model, genjutsu_mapping, history_mapping};

/// Whether the user chose to pay from the plan's unlimited allowance where it applies; asked when a job is priced
/// or sent.
pub type UnlimitedOptIn = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

/// The model constraints the app cached from `model_constraints`, which the length of a request and the estimate
/// keep to, as Create does.
pub type ConstraintsSource = Arc<dyn Fn() -> BoxFuture<'static, ModelConstraintTable> + Send + Sync>;

/// What a quote before upload is priced on: the model, its `mode`, the resolution, the length, the soundtrack and,
/// for a finalize, its draft. Requests that differ only in the rest (the frame, the bitrate, `draft`, which priced
/// the same live) share a quote.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct QuoteKey {
    model: String,
    mode: Option<String>,
    resolution: Option<String>,
    /// The bits of the seconds, as `f64` has no `Hash`.
    duration: Option<u64>,
    generate_audio: Option<bool>,
    draft_job_id: Option<String>,
}

impl QuoteKey {
    fn new(request: &VideoRequest) -> Self {
        Self {
            model: request.model.clone(),
            mode: request.mode.clone(),
            resolution: request.resolution.clone(),
            duration: request.duration.map(f64::to_bits),
            generate_audio: request.generate_audio,
            draft_job_id: request.draft_job_id.clone(),
        }
    }
}

/// What the connection learns as it goes.
#[derive(Default)]
pub(crate) struct State {
    /// The last `models_explore` read; none until `capabilities()`, or when it failed.
    pub(crate) allowance: UnlimitedAllowance,
    /// Set once a `jobs_wait` answer named none of the jobs it was asked about: its shape isn't the one `JobsWait`
    /// reads, so this connection polls each job with `job_status` from then on.
    pub(crate) jobs_wait_unreadable: bool,
    /// `list_voices`, read once for this connection (`voices()`).
    pub(crate) cached_voices: Option<Vec<AccountVoice>>,
}

/// The account connection: Genjutsu and the account's models through the tools of Higgsfield's MCP server, paid
/// with plan credits.
pub struct HiggsfieldMcpProvider {
    pub(crate) account: Arc<HiggsfieldAccount>,
    pub(crate) owner: Option<String>,
    unlimited_opt_in: UnlimitedOptIn,
    constraints: ConstraintsSource,
    pub(crate) state: Mutex<State>,
    /// Before-upload quotes, shared for the connection; failed quotes are dropped for a later retry.
    quotes: QuoteCache<QuoteKey, f64, HiggsfieldError>,
}

/// The largest page each listing asks for.
pub(crate) const HISTORY_PAGE_SIZE: i64 = 24;
const STYLE_PAGE_SIZE: i64 = 50;
const MAX_STYLE_PAGES: usize = 50;
const MOTION_PAGE_SIZE: i64 = 24;
const CREDIT_PAGE_SIZE: i64 = 50;
const UPLOAD_PAGE_SIZE: i64 = 30;

/// What the account connection says when asked to cancel.
pub(crate) fn not_cancellable() -> PortError {
    validation(None, "Higgsfield can't cancel a job from the account connection")
}

/// Assets uploaded before media IDs were kept can't be referenced; the cache expires them.
pub(crate) fn unexpected_upload() -> PortError {
    validation(None, "Unexpected result from the media_upload tool")
}

fn media_id(asset: &RemoteAsset) -> Result<String, PortError> {
    asset.id.clone().ok_or_else(unexpected_upload)
}

/// Lost connections, 5xx and unreadable 2xx replies; 4xx answers mean the call was refused. A call cancelled while
/// out can't tell whether the request went out either.
pub(crate) fn may_have_reached_server(error: &HiggsfieldError) -> bool {
    match error {
        HiggsfieldError::Network | HiggsfieldError::Cancelled => true,
        HiggsfieldError::Server { status } => !(400..500).contains(status),
        _ => false,
    }
}

/// A failed paid call that takes no idempotency key: `OutcomeUnknown` once it may have reached the server.
pub(crate) fn unrepeatable(error: HiggsfieldError) -> PortError {
    if may_have_reached_server(&error) {
        provider(ProviderError::OutcomeUnknown)
    } else {
        port(error)
    }
}

/// A job sent with `use_unlim` is taken as paid from the allowance unless the answer names a price above 0.
/// The tool's description says a refused opt-in is reported, never charged in credits instead, and its
/// `outputSchema` sends `cost` only for `get_cost`; the price is still read in case a live answer differs.
pub(crate) fn unlimited_charge(credits: Option<f64>) -> (bool, Option<Cost>) {
    match credits {
        None | Some(0.0) => (true, None),
        // The job is already accepted: retain its ID and prior quote if the charged amount is unreadable.
        Some(credits) => (false, server_credits(credits, false, 1).ok()),
    }
}

/// Variants are `generate_video`'s `count`: a Genjutsu Style (`execute_preset`) and an operation on a result make
/// one. Refused before anything is sent.
pub(crate) fn check_variants(spec: &JobSpec) -> Result<(), PortError> {
    if spec.variants == 1 {
        return Ok(());
    }
    if (1..=JobSpec::MAXIMUM_VARIANTS).contains(&spec.variants)
        && spec.operation.is_none()
        && spec.mode != Mode::Restyle
    {
        return Ok(());
    }
    Err(validation(
        Some("count"),
        "Variants aren't available for this job.",
    ))
}

pub(crate) fn style_run(spec: &JobSpec, inputs: &RemoteInputs) -> Result<StyleRun, PortError> {
    let (true, Some(preset_id), Some(video)) = (
        spec.model == VideoModel::GENJUTSU,
        spec.preset_id,
        inputs.video.as_ref(),
    ) else {
        return Err(provider(not_available()));
    };
    Ok(StyleRun {
        style_id: genjutsu_mapping::style_id(preset_id),
        video_id: media_id(video)?,
        image_ids: inputs.images.iter().map(media_id).collect::<Result<_, _>>()?,
        prompt: spec.prompt.clone(),
        resolution: spec.resolution.raw_value().to_owned(),
    })
}

fn edit_operation(spec: &JobSpec) -> Result<VideoEditOperation, PortError> {
    account_model::edit_operation(spec).ok_or_else(|| provider(not_available()))
}

fn operation_edit(spec: &JobSpec, inputs: &RemoteInputs) -> Result<VideoEdit, PortError> {
    let operation = edit_operation(spec)?;
    let input = account_model::edit_input(spec, inputs).ok_or_else(|| provider(not_available()))?;
    Ok(VideoEdit { operation, input })
}

impl HiggsfieldMcpProvider {
    /// The connection of `account`. `owner` is `OAuthTokens::owner` of the signed-in account. Nobody opts in to the
    /// unlimited allowance and the built-in constraints hold until `with_unlimited_opt_in` and `with_constraints`
    /// say otherwise.
    pub fn new(account: impl Into<Arc<HiggsfieldAccount>>, owner: Option<String>) -> Self {
        Self {
            account: account.into(),
            owner,
            unlimited_opt_in: Arc::new(|| async { false }.boxed()),
            constraints: Arc::new(|| async { ModelConstraintTable::built_in() }.boxed()),
            state: Mutex::new(State::default()),
            quotes: QuoteCache::default(),
        }
    }

    /// Whether the user chose to pay from the plan's unlimited allowance where it applies.
    #[must_use]
    pub fn with_unlimited_opt_in<F, Fut>(mut self, opt_in: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        self.unlimited_opt_in = Arc::new(move || opt_in().boxed());
        self
    }

    /// The model constraints the app cached from `model_constraints()`, which the length of a request and the
    /// estimate keep to, as Create does.
    #[must_use]
    pub fn with_constraints<F, Fut>(mut self, constraints: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ModelConstraintTable> + Send + 'static,
    {
        self.constraints = Arc::new(move || constraints().boxed());
        self
    }

    /// The account the provider calls.
    pub fn account(&self) -> &HiggsfieldAccount {
        &self.account
    }

    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `models_explore`'s word on the allowance; none when the account has no such tool or the call fails, so a
    /// failure never sends `use_unlim: true`.
    async fn read_allowance(&self) -> UnlimitedAllowance {
        let models: HashSet<String> = account_model::ACCOUNT_MODELS.iter().cloned().collect();
        match self.account.unlimited_allowance(&models).await {
            Ok(allowance) => allowance.unwrap_or_default(),
            Err(_) => UnlimitedAllowance::none(),
        }
    }

    /// The server's tool and model catalogs as JSON, for Debug › Export MCP Tool Catalog. `owner` and every string
    /// in `redacting` are blanked out of it.
    pub async fn export_catalog(&self, redacting: &[String]) -> Result<Vec<u8>, PortError> {
        let mut secrets = redacting.to_vec();
        secrets.extend(self.owner.clone());
        self.account.export_catalog(&secrets).await.map_err(port)
    }

    /// `use_unlim` only for a job queued to be paid that way that still is: opted in, and covered by the allowance.
    async fn video_request(&self, spec: &JobSpec, inputs: &RemoteInputs) -> Result<VideoRequest, PortError> {
        let model = account_model::model(spec).ok_or_else(|| provider(not_available()))?;
        let assets = inputs
            .video
            .iter()
            .map(|asset| (asset, MediaRole::Video))
            .chain(inputs.images.iter().map(|asset| (asset, MediaRole::Image)))
            .chain(inputs.audio.iter().map(|asset| (asset, MediaRole::Audio)));
        let media = assets
            .map(|(asset, role)| media_id(asset).map(|id| VideoMedia::new(role, id)))
            .collect::<Result<Vec<_>, _>>()?;
        let constraints = (self.constraints)().await;
        let mut request =
            account_model::request(spec, &model, media, &account_model::parameters(), &constraints);
        // A finalize sends exactly its four keys: `use_unlim: false` and no `count`.
        if request.draft_job_id.is_some() {
            return Ok(request);
        }
        request.uses_unlimited = spec.uses_unlimited && self.pays_unlimited(spec).await;
        request.count = spec.variants;
        Ok(request)
    }

    /// The quote of an operation before anything is uploaded: every operation is priced by the result's length (an
    /// upscale also by its resolution and frame rate), so the result needn't be named. `Cost::UNKNOWN_CREDITS` when
    /// Higgsfield gives no quote (Topaz). Fails with `not_available` when the tool can't take the spec or the account
    /// has no such tool.
    async fn operation_estimate(&self, spec: &JobSpec) -> Result<Cost, PortError> {
        let operation = edit_operation(spec)?;
        self.account.check_tool(&operation).await.map_err(port)?;
        let length = account_model::source_length(spec);
        let credits = self
            .account
            .operation_cost(&operation, Some(length))
            .await
            .map_err(port)?;
        match credits {
            None => Ok(Cost::UNKNOWN_CREDITS),
            Some(credits) => server_credits(credits, true, 1),
        }
    }

    /// The `get_cost` quote of `request` in credits, asked once per `QuoteKey`: a second estimate of the same job,
    /// even while the first is out, reads the same answer. Free, and submits nothing. `None` when the quote fails,
    /// which isn't kept, so a later estimate asks again.
    async fn quote(&self, request: VideoRequest) -> Result<Option<f64>, PortError> {
        let account = Arc::clone(&self.account);
        let answer = self
            .quotes
            .value(QuoteKey::new(&request), move || async move {
                account.cost(&request).await
            })
            .await;
        match answer {
            Ok(credits) => Ok(Some(credits)),
            Err(HiggsfieldError::Cancelled) => Err(PortError::Cancelled),
            Err(error) => {
                tracing::error!(%error, "get_cost failed, showing the estimate");
                Ok(None)
            }
        }
    }

    /// The Genjutsu Styles, every page. A broken cursor chain fails instead of returning a partial catalog.
    async fn style_presets(&self) -> Result<Vec<StylePreset>, PortError> {
        let mut presets = Vec::new();
        let mut cursor: Option<Value> = None;
        let mut seen = HashSet::new();
        let mut pages = 0;
        loop {
            let page = self
                .account
                .genjutsu_presets(
                    higgsfield::mcp::account::GenjutsuCategory::Styles,
                    cursor.as_ref(),
                    STYLE_PAGE_SIZE,
                )
                .await
                .map_err(port)?;
            presets.extend(page.items.iter().filter_map(genjutsu_mapping::style));
            pages += 1;
            // An empty page that still names a next one would never end.
            cursor = if page.items.is_empty() {
                None
            } else {
                page.next_cursor
            };
            let Some(next) = &cursor else {
                return Ok(presets);
            };
            if !seen.insert(next.to_string()) || pages >= MAX_STYLE_PAGES {
                return Err(provider(ProviderError::Server { status: 200 }));
            }
        }
    }
}

impl std::fmt::Debug for HiggsfieldMcpProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HiggsfieldMcpProvider")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl GenerationProvider for HiggsfieldMcpProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Account
    }

    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }

    /// Re-reads `tools/list`; without every tool a job needs, no mode is offered. Restyle runs as a Genjutsu Style.
    /// Edit and Extend, and each model parameter, run behind their switches. Also reads the unlimited allowance.
    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        let can_generate = self.account.refresh_tools().await.map_err(port)?;
        let mut modes = if can_generate {
            account_model::generate_modes()
        } else {
            BTreeSet::new()
        };
        if self.account.can_run_styles() {
            modes.insert(Mode::Restyle);
        }
        let allowance = if can_generate {
            self.read_allowance().await
        } else {
            UnlimitedAllowance::none()
        };
        let unlimited = account_model::unlimited(&allowance, &account_model::generate_modes());
        self.state().allowance = allowance;
        let mut capabilities = Capabilities::new(
            modes,
            Resolution::ALL.iter().copied().collect(),
            CostUnit::Credits,
            None,
        );
        capabilities.models = if can_generate {
            VideoModel::all()
                .into_iter()
                .filter(account_model::runs)
                .collect()
        } else {
            BTreeSet::from([VideoModel::GENJUTSU])
        };
        capabilities.parameters = if can_generate {
            account_model::parameters()
        } else {
            BTreeSet::new()
        };
        capabilities.unlimited = unlimited;
        capabilities.max_variants = if can_generate {
            JobSpec::MAXIMUM_VARIANTS
        } else {
            1
        };
        Ok(capabilities)
    }

    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        self.style_presets().await
    }

    /// The price before anything is uploaded. A model sent a length is quoted by `get_cost` without media
    /// (`account_model::quote_request`), which is the exact price; a model billed by its source (Genjutsu, Edit) and
    /// a Genjutsu Style can be quoted only with the uploaded source, so they get the catalog's estimate, as does a
    /// quote that fails; `Cost::UNKNOWN_CREDITS` when there is no rate either. A generation of several variants costs
    /// one variant's price that many times.
    async fn estimate(&self, spec: &JobSpec) -> Result<Cost, PortError> {
        check_variants(spec)?;
        if spec.operation.is_some() {
            return self.operation_estimate(spec).await;
        }
        if spec.mode == Mode::Restyle {
            return Ok(Cost::credits(
                Decimal::from(account_model::style_credits(spec)),
                true,
            ));
        }
        let model = account_model::model(spec).ok_or_else(|| provider(not_available()))?;
        let constraints = (self.constraints)().await;
        if let Some(request) = account_model::quote_request(spec, &model, &constraints)
            && let Some(credits) = self.quote(request).await?
            // A malformed price falls back to the estimate.
            && let Ok(quoted) = server_credits(credits, false, spec.variants)
        {
            return Ok(quoted);
        }
        let Some(credits) = account_model::estimated_credits(spec, &constraints) else {
            return Ok(Cost::UNKNOWN_CREDITS);
        };
        Ok(Cost::credits(Decimal::from(credits), true).times(spec.variants))
    }

    /// A generation of several variants is quoted for one variant, then priced that many times: the variants share
    /// every setting, and `get_cost` leaves `count` out of the price (live, 2026-10-15: `count` 2 quoted the same as
    /// 1), so the quote is one variant's.
    async fn server_estimate(
        &self,
        spec: &JobSpec,
        inputs: &RemoteInputs,
    ) -> Result<Option<Cost>, PortError> {
        check_variants(spec)?;
        let credits = if spec.operation.is_some() {
            let operation = operation_edit(spec, inputs)?.operation;
            self.account
                .operation_cost(&operation, Some(account_model::source_length(spec)))
                .await
                .map_err(port)?
        } else if spec.mode == Mode::Restyle {
            let run = style_run(spec, inputs)?;
            self.account.style_cost(&run).await.map_err(port)?
        } else {
            let mut request = self.video_request(spec, inputs).await?;
            request.count = 1;
            Some(self.account.cost(&request).await.map_err(port)?)
        };
        // Up: a price must never look lower than what the server charges.
        credits
            .map(|credits| server_credits(credits, false, spec.variants))
            .transpose()
    }

    async fn charge(&self, job: &Job) -> Result<Option<Cost>, PortError> {
        self.topaz_charge(job).await
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        let path = file_path(&file.file_url)?;
        let media = self
            .account
            .upload(&path, &file.content_type)
            .await
            .map_err(port)?;
        Ok(RemoteAsset {
            url: media.url,
            id: Some(media.id),
        })
    }

    /// A linked file of at most 50 MB is imported by its link with `media_import_url` (`url`, `type`; a confirmed
    /// `media_id` back), so it isn't uploaded. A larger file, a refused import (the host may block the server's
    /// fetch) or an account without the tool uploads it as usual.
    async fn linked_asset(
        &self,
        origin: &Url,
        kind: MediaLinkKind,
        byte_count: i64,
    ) -> Result<Option<RemoteAsset>, PortError> {
        if !is_https(origin) || byte_count <= 0 || byte_count > HiggsfieldAccount::IMPORT_LIMIT {
            return Ok(None);
        }
        match self.account.import_media(origin, kind.raw_value()).await {
            Ok(id) => Ok(Some(RemoteAsset {
                url: origin.clone(),
                id: Some(id),
            })),
            Err(HiggsfieldError::Validation { .. } | HiggsfieldError::ToolUnavailable(_)) => Ok(None),
            Err(error) => Err(port(error)),
        }
    }

    async fn server_asset(&self, media: &ServerMedia) -> Result<Option<RemoteAsset>, PortError> {
        Ok(self.reused_asset(media))
    }

    /// `generate_video` takes no idempotency key, so sending it again could pay twice: any failure once the call may
    /// have reached the server fails with `ProviderError::OutcomeUnknown`, never an error the engine retries. A
    /// future dropped before it is polled sends nothing. Every further job the answer names, one per variant, comes
    /// back in `RemoteJob::variants`.
    async fn submit(
        &self,
        spec: &JobSpec,
        inputs: &RemoteInputs,
        _idempotency_key: Uuid,
    ) -> Result<RemoteJob, PortError> {
        check_variants(spec)?;
        let mut uses_unlimited = false;
        let mut is_native_draft = false;
        let sent: Result<Generation, HiggsfieldError> = if spec.operation.is_some() {
            let edit = operation_edit(spec, inputs)?;
            self.account.run_edit(&edit).await
        } else if spec.mode == Mode::Restyle {
            let run = style_run(spec, inputs)?;
            self.account.run_style(&run).await
        } else {
            let request = self.video_request(spec, inputs).await?;
            uses_unlimited = request.uses_unlimited;
            is_native_draft = request.draft;
            self.account.generate(&request).await
        };
        let generation = sent.map_err(unrepeatable)?;
        let mut remote = RemoteJob::new(generation.id);
        remote.correlation_id.clone_from(&generation.request_id);
        if uses_unlimited {
            (remote.paid_with_unlimited, remote.charged) = unlimited_charge(generation.credits);
        }
        remote.is_native_draft = is_native_draft;
        remote.variants = generation
            .variant_ids
            .into_iter()
            .map(|id| {
                let mut variant = RemoteJob::new(id);
                variant.correlation_id.clone_from(&generation.request_id);
                variant.is_native_draft = is_native_draft;
                variant
            })
            .collect();
        Ok(remote)
    }

    /// Never: `generate_video` has no idempotency key. Fails like `submit` when the tool is missing.
    async fn submit_is_idempotent(&self, spec: &JobSpec) -> Result<bool, PortError> {
        if spec.operation.is_some() {
            let operation = edit_operation(spec)?;
            self.account.check_tool(&operation).await.map_err(port)?;
            return Ok(false);
        }
        self.account.check_generate().await.map_err(port)?;
        Ok(false)
    }

    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        let status = self.account.status(&job.request_id).await.map_err(port)?;
        Ok(remote_status(status))
    }

    fn status_batch_limit(&self) -> usize {
        HiggsfieldAccount::JOBS_WAIT_LIMIT
    }

    /// One `jobs_wait` long-poll of `jobs`. `None`, so the engine asks `job_status` for each job, when the account
    /// has no usable `jobs_wait`, the call fails, or the answer can't be read: the shape is unconfirmed (issue #37),
    /// so a wrong guess costs one extra call, never a stalled job. After an unreadable answer it isn't asked again.
    async fn statuses(&self, jobs: &[RemoteJob]) -> Result<Option<BatchStatus>, PortError> {
        if self.state().jobs_wait_unreadable || jobs.is_empty() {
            return Ok(None);
        }
        let ids: Vec<String> = jobs.iter().map(|job| job.request_id.clone()).collect();
        let answer = match self.account.wait_for_jobs(&ids).await {
            Ok(answer) => answer,
            Err(HiggsfieldError::Cancelled) => return Err(PortError::Cancelled),
            Err(error) => {
                tracing::error!(%error, "jobs_wait failed, polling each job");
                return Ok(None);
            }
        };
        let Some(answer) = answer else {
            return Ok(None);
        };
        if answer.statuses.is_empty() {
            self.state().jobs_wait_unreadable = true;
            tracing::error!("jobs_wait answered in an unknown shape; polling each job from now on");
            return Ok(None);
        }
        Ok(Some(BatchStatus {
            statuses: answer
                .statuses
                .into_iter()
                .map(|(id, status)| (id, remote_status(status)))
                .collect(),
            poll_after: answer
                .poll_after_seconds
                .and_then(time::Duration::checked_seconds_f64),
        }))
    }

    /// No tool cancels a generation.
    async fn cancel(&self, _job: &RemoteJob) -> Result<(), PortError> {
        Err(not_cancellable())
    }

    /// `None` when the account exposes no balance tool, as the protocol defines for "can't report". While the
    /// account has an unlimited allowance, reads it again too: this runs after each job, and jobs spend it.
    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        let available = self.state().allowance.is_available;
        if available {
            let allowance = self.read_allowance().await;
            self.state().allowance = allowance;
        }
        let Some(balance) = self.account.balance().await.map_err(port)? else {
            return Ok(None);
        };
        // Down: credits left must never look higher than they are, or a job the server refuses looks affordable.
        let credits =
            whole_number(balance.credits.floor()).ok_or(provider(ProviderError::Server { status: 200 }))?;
        Ok(Some(Balance::Credits {
            credits,
            plan: balance.plan,
        }))
    }

    /// Opted in, and the last read of the allowance covers the spec's `generate_video` model. A Genjutsu Style runs
    /// through `execute_preset`, which has no such option, and a finalize always sends `use_unlim: false`.
    async fn pays_unlimited(&self, spec: &JobSpec) -> bool {
        if spec.operation.is_some() || spec.mode == Mode::Restyle || spec.draft_job_id.is_some() {
            return false;
        }
        if !(self.unlimited_opt_in)().await {
            return false;
        }
        let Some(model) = account_model::model(spec) else {
            return false;
        };
        self.state().allowance.covers(&model)
    }
}

#[async_trait]
impl GenerationHistory for HiggsfieldMcpProvider {
    /// Finished videos and images, of the selected workspace; other media types and models are left out. Each
    /// stream pages on its own, packed into one cursor, so images don't hold back videos or the other way round.
    async fn history(&self, after: Option<HistoryCursor>) -> Result<HistoryPage, PortError> {
        let (video_cursor, image_cursor) = history_mapping::split_cursor(after.as_ref());
        // Past its last page a stream isn't asked again; the first page asks both, videos first.
        let video_page = if after.is_none() || video_cursor.is_some() {
            Some(
                self.account
                    .generations(
                        Some(GenerationType::Video),
                        true,
                        video_cursor.as_ref(),
                        HISTORY_PAGE_SIZE,
                    )
                    .await
                    .map_err(port)?,
            )
        } else {
            None
        };
        let image_page = if after.is_none() || image_cursor.is_some() {
            Some(
                self.account
                    .generations(
                        Some(GenerationType::Image),
                        true,
                        image_cursor.as_ref(),
                        HISTORY_PAGE_SIZE,
                    )
                    .await
                    .map_err(port)?,
            )
        } else {
            None
        };
        let videos = video_page.as_ref().map(history_mapping::page);
        let images = image_page.as_ref().map(history_mapping::page);
        let next_cursor = history_mapping::joined_cursor(
            videos.as_ref().and_then(|page| page.next_cursor.as_ref()),
            images.as_ref().and_then(|page| page.next_cursor.as_ref()),
        );
        Ok(HistoryPage {
            items: videos.map(|page| page.items).unwrap_or_default(),
            images: images.map(|page| page.images).unwrap_or_default(),
            next_cursor,
        })
    }
}

#[async_trait]
impl ModelConstraintsSource for HiggsfieldMcpProvider {
    /// `models_explore` `get` of each `generate_video` model; `None` without `models_explore`.
    async fn model_constraints(&self) -> Result<Option<ModelConstraintTable>, PortError> {
        let limits = self
            .account
            .model_limits(&account_model::ACCOUNT_MODELS)
            .await
            .map_err(port)?;
        Ok(limits.as_ref().map(account_model::constraints))
    }
}

/// The next cursor of a page: none after an empty page, which would never end if it named one.
pub(crate) fn page_cursor(is_empty: bool, cursor: Option<&Value>) -> Option<HistoryCursor> {
    if is_empty {
        None
    } else {
        cursor.and_then(history_mapping::cursor)
    }
}

#[async_trait]
impl MotionLibrary for HiggsfieldMcpProvider {
    async fn motion_presets(
        &self,
        gallery: MotionShelf,
        after: Option<HistoryCursor>,
    ) -> Result<MotionPresetPage, PortError> {
        let Some(category) = genjutsu_mapping::category(gallery) else {
            let page = self.uploads(UploadKind::Video, after).await?;
            return Ok(MotionPresetPage {
                items: page.items.iter().map(account_mapping::clip).collect(),
                next_cursor: page.next_cursor,
            });
        };
        let cursor = after.as_ref().and_then(history_mapping::cursor_value);
        let page = self
            .account
            .genjutsu_presets(category, cursor.as_ref(), MOTION_PAGE_SIZE)
            .await
            .map_err(port)?;
        Ok(MotionPresetPage {
            items: page.items.iter().filter_map(genjutsu_mapping::motion).collect(),
            next_cursor: page_cursor(page.items.is_empty(), page.next_cursor.as_ref()),
        })
    }
}

#[async_trait]
impl GenerationLikes for HiggsfieldMcpProvider {
    async fn set_liked(&self, request_id: String, is_liked: bool) -> Result<(), PortError> {
        self.account.set_liked(&request_id, is_liked).await.map_err(port)
    }
}

#[async_trait]
impl CreditHistory for HiggsfieldMcpProvider {
    async fn credit_history(&self, after: Option<HistoryCursor>) -> Result<CreditPage, PortError> {
        let cursor = after.as_ref().and_then(history_mapping::cursor_value);
        let page = self
            .account
            .transactions(cursor.as_ref(), CREDIT_PAGE_SIZE)
            .await
            .map_err(port)?;
        Ok(CreditPage {
            items: page.items.iter().map(account_mapping::entry).collect(),
            next_cursor: page_cursor(page.items.is_empty(), page.next_cursor.as_ref()),
        })
    }
}

#[async_trait]
impl AccountUploads for HiggsfieldMcpProvider {
    async fn uploads(&self, kind: UploadKind, after: Option<HistoryCursor>) -> Result<UploadPage, PortError> {
        let cursor = after.as_ref().and_then(history_mapping::cursor_value);
        let page = self
            .account
            .uploads(
                account_mapping::asset_kind(kind),
                cursor.as_ref(),
                UPLOAD_PAGE_SIZE,
            )
            .await
            .map_err(port)?;
        Ok(UploadPage {
            items: page.items.iter().map(account_mapping::file).collect(),
            next_cursor: page_cursor(page.items.is_empty(), page.next_cursor.as_ref()),
        })
    }
}
