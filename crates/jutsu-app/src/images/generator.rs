//! What Generate on the Image side asks for and runs (`ReferenceGenerator.swift`, `ReferenceGenerator+Runs.swift`):
//! the form, its price, the runs with their images. Image jobs never enter the job engine: the runs are polled here
//! and their prices go into the `ImageSpendLog`, which the daily cap reads.
//!
//! Money: a request's price is written before it is sent, and it isn't sent when that write fails. Every job the
//! answer names goes into the `ImageJobLog` and is followed until it ends, after a lost sign-in, a switch of account
//! or a relaunch too (`resume`). Only an answer that the job failed, was refused or was cancelled takes the price
//! off; a status that can't be read leaves it counted.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use jutsu_core::{
    AgentOrigin, Backoff, BudgetError, Cost, ImageCharge, ImageChargePurpose, ImageJobStatus, ImageProvider,
    ImageRequest, LocalImage, MaterialRemote, Mode, PixelSize, PortError, ProviderError, ProviderKind,
    RemoteAsset, RemoteJob, SpendStatus,
};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::form::{ImageForm, ImageRequestDraft, cap_confirmation};
use super::job_log::{ImageJobLog, ImageJobRecord, ImageJobState};
use super::spend_log::ImageSpendLog;
use crate::connection::UserMessage;
use crate::queue::QueueConfirmation;

/// Waits between polls; tests pass one that only yields.
pub type SleepFn = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;
/// Today's spend and the cap of a connection kind, read right before each request is sent; `None` without a cap.
pub type SpendStatusReader =
    Arc<dyn Fn(ProviderKind) -> BoxFuture<'static, Result<Option<SpendStatus>, BudgetError>> + Send + Sync>;
/// Uploads the image to edit.
pub type UploadFn =
    Arc<dyn Fn(LocalImage) -> BoxFuture<'static, Result<RemoteAsset, PortError>> + Send + Sync>;
/// Called with each run's images as they arrive, to save them.
pub type ImagesHandler = Arc<dyn Fn(Vec<GeneratedImage>) + Send + Sync>;
/// Called when Higgsfield turned a poll down because the sign-in no longer works.
pub type SignInHandler = Arc<dyn Fn() + Send + Sync>;

pub const COULD_NOT_MAKE: &str = "Higgsfield couldn't make the image.";
pub const WAITS_FOR_ACCOUNT: &str =
    "Followed again when the account that sent it is connected. Its price still counts.";
const REFUSED_BY_CHECK: &str = "Higgsfield's content check refused the prompt.";

/// An image made in this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub id: Uuid,
    /// Where the provider serves it.
    pub url: Url,
    pub prompt: String,
    pub model: String,
    pub aspect_ratio: Option<String>,
    /// The copy saved in the output folder's `Images` folder; `None` until it is saved, or when saving failed.
    pub file: Option<PathBuf>,
    pub remote: Option<MaterialRemote>,
    /// The exact request, as JSON.
    pub parameters: Option<Vec<u8>>,
    pub input_images: Vec<LocalImage>,
    pub variant_group_id: Option<Uuid>,
    /// An agent's name for the images; their file name uses it instead of the prompt.
    pub title: Option<String>,
    /// Added to Create's references.
    pub is_picked: bool,
}

impl GeneratedImage {
    pub fn new(url: Url, prompt: &str, model: &str, aspect_ratio: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            url,
            prompt: prompt.to_owned(),
            model: model.to_owned(),
            aspect_ratio,
            file: None,
            remote: None,
            parameters: None,
            input_images: Vec::new(),
            variant_group_id: None,
            title: None,
            is_picked: false,
        }
    }
}

/// Where a request is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunState {
    Running,
    Finished,
    Failed(String),
    Canceled,
    /// Following it stopped before it ended: the sign-in expired, its status couldn't be read, or another account
    /// was connected. Its jobs and price are kept, and it is followed again (`resume`).
    Interrupted(String),
    /// An agent's request the spending guard stopped before anything was sent: only the person sends it, after
    /// seeing why (`AgentImageQueue::approve`).
    Held(String),
}

/// One Generate request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRun {
    pub id: Uuid,
    pub model: String,
    pub count: i64,
    pub state: RunState,
    /// The agent that asked for it (`create_image`); `None` for the person's own.
    pub agent: Option<AgentOrigin>,
    /// An agent's request: its price as the guard saw it, for the person's confirmation.
    pub price: Option<Cost>,
    /// An agent's name for the request, which its row starts with.
    pub name: Option<String>,
}

impl ImageRun {
    /// The person's own request.
    pub fn new(id: Uuid, model: String, count: i64, state: RunState) -> Self {
        Self {
            id,
            model,
            count,
            state,
            agent: None,
            price: None,
            name: None,
        }
    }

    /// Its row stays when a new request replaces the rows of the ones that are over.
    fn keeps_row(&self) -> bool {
        matches!(self.state, RunState::Running | RunState::Held(_))
    }

    /// "Soul 2 · 4 images", after the agent's name for it when it gave one.
    pub fn title(&self) -> String {
        let title = super::form::run_title(&self.model, self.count);
        match &self.name {
            Some(name) => format!("{name} · {title}"),
            None => title,
        }
    }
}

/// How an agent's submit went (`AgentImageSubmit.Outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSubmitOutcome {
    /// Stopped before the submit went out.
    NotSent,
    /// The provider turned it down, so it never started.
    Refused,
    /// Cut off or unanswered: it may have started.
    Unknown,
    Accepted,
}

/// The spending guard, run once the request is built, right before its price is written and it is sent: the price
/// to send it at, or `None` when the guard held or failed it and nothing may go out.
pub type AgentClearFn = Arc<dyn Fn(ImageRequest) -> BoxFuture<'static, Option<Cost>> + Send + Sync>;
/// Told how the submit went.
pub type AgentAnsweredFn = Arc<dyn Fn(AgentSubmitOutcome) -> BoxFuture<'static, ()> + Send + Sync>;
/// Called when a run ends or stops being followed, with how: what `AgentImageQueue` records for an agent.
pub type RunEndedHandler = Arc<dyn Fn(Uuid, &RunState) + Send + Sync>;

/// What an agent's request adds to a submit (`AgentImageQueue`): its spending guard, and how the submit went.
#[derive(Clone)]
pub struct AgentImageSubmit {
    pub clear: AgentClearFn,
    pub answered: AgentAnsweredFn,
}

/// What Generate did about today's cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaidStart {
    Started,
    /// It would take today past the cap, or its price isn't known under one: nothing was sent.
    NeedsConfirmation(QueueConfirmation),
    /// Nothing to send: the form can't run, or another connection came first.
    NotStarted,
}

#[derive(Default)]
struct GeneratorState {
    form: ImageForm,
    /// The price of the request as it stands; `None` while it is being read or can't be.
    price: Option<Cost>,
    price_failure: Option<String>,
    runs: Vec<ImageRun>,
    images: Vec<GeneratedImage>,
    /// The images picked to become references, in the order they were picked.
    selection: Vec<Uuid>,
    /// The uploads and submits, by run. Not cut off by a new connection: a submit may already be paid for.
    submits: HashMap<Uuid, CancellationToken>,
    /// The polls, by run.
    followers: HashMap<Uuid, CancellationToken>,
    /// Runs the user cancelled while they were followed.
    canceling: HashSet<Uuid>,
    /// Goes up with each connection (`reset`), so an upload or a price of the old one isn't kept.
    generation: u64,
    /// The last input uploaded on this connection, so pricing and Generate don't upload it again.
    uploaded: Option<(LocalImage, RemoteAsset)>,
}

/// The image form and its runs.
pub struct ReferenceGenerator {
    spend: Arc<ImageSpendLog>,
    job_log: Arc<ImageJobLog>,
    spend_status: SpendStatusReader,
    sleep: SleepFn,
    state: Mutex<GeneratorState>,
    on_images: Mutex<Option<ImagesHandler>>,
    on_sign_in_expired: Mutex<Option<SignInHandler>>,
    on_run_ended: Mutex<Option<RunEndedHandler>>,
    submitted: Notify,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for ReferenceGenerator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferenceGenerator")
            .finish_non_exhaustive()
    }
}

/// How one round of polls went.
enum PollRound {
    Read,
    /// Following stopped: cancelled, by the user or for another connection.
    Stopped,
    /// A status that can't be read, for a reason waiting doesn't fix.
    Interrupted(PortError),
}

/// Runs `work` off the async threads: the logs write their files.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work)
        .await
        .inspect_err(|error| tracing::error!(%error, "a log write didn't finish"))
        .ok()
}

/// A poll that failed for a reason waiting can fix.
fn is_transient(error: &PortError) -> bool {
    matches!(
        error,
        PortError::Provider(
            ProviderError::Network
                | ProviderError::Server { .. }
                | ProviderError::ModelUnavailable
                | ProviderError::ConcurrencyLimit { .. }
        )
    )
}

/// The server turned the request down, so it never started.
pub fn was_refused(error: &PortError) -> bool {
    matches!(
        error,
        PortError::Provider(
            ProviderError::Unauthorized
                | ProviderError::InsufficientFunds
                | ProviderError::ConcurrencyLimit { .. }
                | ProviderError::Validation { .. }
                | ProviderError::ModelUnavailable
                | ProviderError::Rejected
        )
    )
}

/// What to tell about a failed request.
pub fn message(error: &PortError, kind: ProviderKind) -> String {
    UserMessage::from_error(error, Some(kind))
        .map_or_else(|| COULD_NOT_MAKE.to_owned(), |message| message.message)
}

/// Why a run stopped being followed.
pub fn interrupted(error: &PortError, kind: ProviderKind) -> String {
    if *error == PortError::Provider(ProviderError::Unauthorized) {
        return "Connect Higgsfield again to keep following this request. Its price still counts.".to_owned();
    }
    format!(
        "Can't read where this request is: {} It's followed again after the next sign-in or launch, and its price still counts.",
        message(error, kind)
    )
}

impl ReferenceGenerator {
    pub fn new(
        spend: Arc<ImageSpendLog>,
        job_log: Arc<ImageJobLog>,
        spend_status: SpendStatusReader,
        sleep: SleepFn,
    ) -> Arc<Self> {
        Arc::new(Self {
            spend,
            job_log,
            spend_status,
            sleep,
            state: Mutex::new(GeneratorState::default()),
            on_images: Mutex::new(None),
            on_sign_in_expired: Mutex::new(None),
            on_run_ended: Mutex::new(None),
            submitted: Notify::new(),
            changes: watch::Sender::new(0),
        })
    }

    /// Sleeps for real between polls.
    pub fn real_sleep() -> SleepFn {
        Arc::new(|duration| Box::pin(tokio::time::sleep(duration)))
    }

    fn locked(&self) -> MutexGuard<'_, GeneratorState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn spend(&self) -> &Arc<ImageSpendLog> {
        &self.spend
    }

    pub fn job_log(&self) -> &Arc<ImageJobLog> {
        &self.job_log
    }

    /// Today's spend and the cap of a connection kind; `None` without a cap.
    pub async fn spend_status(&self, kind: ProviderKind) -> Result<Option<SpendStatus>, BudgetError> {
        (self.spend_status)(kind).await
    }

    /// Bumps whenever the form, the price, the runs or the images change.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(crate) fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    pub fn set_on_images(&self, handler: ImagesHandler) {
        *self.on_images.lock().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    pub fn set_on_sign_in_expired(&self, handler: SignInHandler) {
        *self
            .on_sign_in_expired
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    pub fn set_on_run_ended(&self, handler: RunEndedHandler) {
        *self.on_run_ended.lock().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    // MARK: Reads

    pub fn form(&self) -> ImageForm {
        self.locked().form.clone()
    }

    /// Changes the form; the price is read again by whoever shows it.
    pub fn edit_form<T>(&self, edit: impl FnOnce(&mut ImageForm) -> T) -> T {
        let result = edit(&mut self.locked().form);
        self.changed();
        result
    }

    pub fn price(&self) -> Option<Cost> {
        self.locked().price.clone()
    }

    pub fn price_failure(&self) -> Option<String> {
        self.locked().price_failure.clone()
    }

    pub fn runs(&self) -> Vec<ImageRun> {
        self.locked().runs.clone()
    }

    pub fn images(&self) -> Vec<GeneratedImage> {
        self.locked().images.clone()
    }

    pub fn is_running(&self) -> bool {
        self.locked()
            .runs
            .iter()
            .any(|run| run.state == RunState::Running)
    }

    /// Requests whose upload or submit is out; quitting waits for them (`finish_submits`).
    pub fn sending_count(&self) -> usize {
        self.locked().submits.len()
    }

    /// Requests being sent or followed until their images arrive.
    pub fn active_count(&self) -> usize {
        let state = self.locked();
        state.submits.len() + state.followers.len()
    }

    // MARK: Requests

    /// `draft` as a request, its inputs uploaded in order through `upload`, except the image uploaded last when it is
    /// one of them.
    pub async fn request(
        &self,
        draft: &ImageRequestDraft,
        upload: &UploadFn,
    ) -> Result<ImageRequest, PortError> {
        let mut inputs = Vec::with_capacity(draft.inputs.len());
        for input in &draft.inputs {
            let (uploaded, generation) = {
                let state = self.locked();
                (
                    state
                        .uploaded
                        .as_ref()
                        .filter(|(image, _)| image == input)
                        .map(|(_, asset)| asset.clone()),
                    state.generation,
                )
            };
            if let Some(asset) = uploaded {
                inputs.push(asset);
                continue;
            }
            let asset = upload(input.clone()).await?;
            // An upload that outlived its connection is the old account's media: never offered to the new one.
            let mut state = self.locked();
            if state.generation == generation {
                state.uploaded = Some((input.clone(), asset.clone()));
            }
            inputs.push(asset);
        }
        Ok(ImageRequest {
            model: draft.model.id.clone(),
            prompt: draft.prompt.clone(),
            aspect_ratio: draft.aspect_ratio.clone(),
            count: draft.count,
            input_images: inputs,
        })
    }

    /// Reads the price of the request as it stands: the provider's quote, or the model's list price when the
    /// provider can't give one.
    pub async fn refresh_price(
        &self,
        provider: &Arc<dyn ImageProvider>,
        source: Option<PixelSize>,
        mode: Mode,
        upload: &UploadFn,
    ) {
        let (draft, generation) = {
            let mut state = self.locked();
            state.price = None;
            state.price_failure = None;
            (state.form.draft(source, mode), state.generation)
        };
        let Some(draft) = draft else {
            self.changed();
            return;
        };
        let quote = match self.request(&draft, upload).await {
            Ok(request) => provider.image_estimate(&request).await,
            Err(error) => Err(error),
        };
        {
            let mut state = self.locked();
            if state.generation != generation {
                return;
            }
            match quote {
                Ok(quote) => state.price = Some(quote),
                Err(PortError::Cancelled) => return,
                Err(error) => {
                    state.price = state.form.list_price();
                    state.price_failure =
                        UserMessage::from_error(&error, Some(provider.kind())).map(|message| message.message);
                }
            }
        }
        self.changed();
    }

    /// Submits the request as it stands and follows it until its images arrive. Its price counts toward today's
    /// spend from the submit; a request that is refused, fails or is cancelled while queued is taken off again.
    /// Unless `over_cap_approved`, today's spend is read first, and a price that takes today past the cap, or isn't
    /// known under one, sends nothing and asks.
    /// - `price`: the price shown; `None` counts as nothing until a quote exists.
    pub async fn generate(
        self: &Arc<Self>,
        provider: Arc<dyn ImageProvider>,
        source: Option<PixelSize>,
        mode: Mode,
        price: Option<Cost>,
        over_cap_approved: bool,
        upload: UploadFn,
    ) -> PaidStart {
        let (draft, generation) = {
            let state = self.locked();
            (state.form.draft(source, mode), state.generation)
        };
        let Some(draft) = draft else {
            return PaidStart::NotStarted;
        };
        match (self.spend_status)(provider.kind()).await {
            Ok(status) => {
                // Another account connected while today's spend was read: the request was the old one's.
                if self.locked().generation != generation {
                    return PaidStart::NotStarted;
                }
                if !over_cap_approved
                    && let Some(status) = status
                    && let Some(question) =
                        cap_confirmation(price.as_ref(), status.committed_today, status.cap.as_ref())
                {
                    return PaidStart::NeedsConfirmation(question);
                }
            }
            Err(error) => {
                self.locked().runs.insert(
                    0,
                    ImageRun::new(
                        Uuid::new_v4(),
                        draft.model.title.clone(),
                        draft.count,
                        RunState::Failed(error.to_string()),
                    ),
                );
                self.changed();
                return PaidStart::NotStarted;
            }
        }
        let id = Uuid::new_v4();
        let cost = price.unwrap_or_else(|| match provider.kind() {
            ProviderKind::ApiKey => Cost::usd(Decimal::ZERO, true),
            ProviderKind::Account => Cost::UNKNOWN_CREDITS,
        });
        let token = CancellationToken::new();
        {
            let mut state = self.locked();
            // A new request replaces the rows of the ones that are over; their images stay.
            state.runs.retain(ImageRun::keeps_row);
            state.runs.insert(
                0,
                ImageRun::new(id, draft.model.title.clone(), draft.count, RunState::Running),
            );
            state.submits.insert(id, token.clone());
        }
        self.changed();
        self.spawn_submit(id, draft, cost, provider, upload, token, None);
        PaidStart::Started
    }

    /// Sends an agent's request through the same submit as Generate, with its spending guard (`AgentImageQueue`):
    /// the guard runs once the request is built and decides the price it goes out at. Its run is shown by the
    /// queue.
    pub fn submit_for_agent(
        self: &Arc<Self>,
        id: Uuid,
        draft: ImageRequestDraft,
        provider: Arc<dyn ImageProvider>,
        upload: UploadFn,
        agent: AgentImageSubmit,
    ) {
        let token = CancellationToken::new();
        self.locked().submits.insert(id, token.clone());
        self.changed();
        self.spawn_submit(
            id,
            draft,
            Cost::UNKNOWN_CREDITS,
            provider,
            upload,
            token,
            Some(agent),
        );
    }

    #[allow(clippy::too_many_arguments, reason = "the submit's parts, handed to its task")]
    fn spawn_submit(
        self: &Arc<Self>,
        id: Uuid,
        draft: ImageRequestDraft,
        cost: Cost,
        provider: Arc<dyn ImageProvider>,
        upload: UploadFn,
        token: CancellationToken,
        agent: Option<AgentImageSubmit>,
    ) {
        let generator = self.clone();
        tokio::spawn(async move {
            generator
                .submit(id, draft, cost, provider, upload, token, agent)
                .await;
            generator.locked().submits.remove(&id);
            generator.submitted.notify_waiters();
            generator.changed();
        });
    }

    /// Whether the run's upload or submit is out.
    pub fn is_submitting(&self, id: Uuid) -> bool {
        self.locked().submits.contains_key(&id)
    }

    /// Shows a run, in place of the one with its ID or first in the list (an agent's request, `AgentImageQueue`).
    pub fn show_run(&self, run: ImageRun) {
        {
            let mut state = self.locked();
            if let Some(existing) = state.runs.iter_mut().find(|existing| existing.id == run.id) {
                *existing = run;
            } else {
                state.runs.insert(0, run);
            }
        }
        self.changed();
    }

    /// Changes the run with that ID, if it is shown.
    pub fn update_run(&self, id: Uuid, change: impl FnOnce(&mut ImageRun)) {
        {
            let mut state = self.locked();
            let Some(run) = state.runs.iter_mut().find(|run| run.id == id) else {
                return;
            };
            change(run);
        }
        self.changed();
    }

    /// - `agent`: an agent's request: its spending guard runs once the request is built, and is told how the submit
    ///   went; `None` for the person's own.
    #[allow(clippy::too_many_arguments, reason = "the submit's parts")]
    async fn submit(
        self: &Arc<Self>,
        id: Uuid,
        draft: ImageRequestDraft,
        cost: Cost,
        provider: Arc<dyn ImageProvider>,
        upload: UploadFn,
        token: CancellationToken,
        agent: Option<AgentImageSubmit>,
    ) {
        let kind = provider.kind();
        let request = tokio::select! {
            request = self.request(&draft, &upload) => request,
            () = token.cancelled() => return self.finish(id, RunState::Canceled),
        };
        let request = match request {
            Ok(request) if !token.is_cancelled() => request,
            Ok(_) | Err(PortError::Cancelled) => return self.finish(id, RunState::Canceled),
            Err(error) => return self.finish(id, RunState::Failed(message(&error, kind))),
        };
        let mut cost = cost;
        if let Some(agent) = &agent {
            // The guard holds or fails the run itself when it lets nothing through.
            let Some(cleared) = (agent.clear)(request.clone()).await else {
                return;
            };
            cost = cleared;
            if token.is_cancelled() {
                (agent.answered)(AgentSubmitOutcome::NotSent).await;
                return self.finish(id, RunState::Canceled);
            }
        }
        let charge = ImageCharge::new(
            id,
            OffsetDateTime::now_utc(),
            cost,
            kind,
            draft.model.title.clone(),
            draft.count,
            Some(ImageChargePurpose::ReferenceImages),
        );
        let spend = self.spend.clone();
        let added = match blocking(move || spend.add(charge)).await {
            Some(Ok(())) => None,
            Some(Err(error)) => Some(RunState::Failed(ImageSpendLog::not_sent(&error))),
            None => Some(RunState::Failed(COULD_NOT_MAKE.to_owned())),
        };
        if let Some(failed) = added {
            if let Some(agent) = &agent {
                (agent.answered)(AgentSubmitOutcome::NotSent).await;
            }
            return self.finish(id, failed);
        }
        // The submit itself isn't cut off: once it went out it may be paid for.
        let jobs = match provider.submit_images(&request, id).await {
            Ok(jobs) => jobs,
            Err(error) => {
                // A submit cut off or unanswered may have started the request, so its price keeps counting.
                let spend = self.spend.clone();
                let refused = was_refused(&error);
                blocking(move || {
                    if refused {
                        spend.remove(id);
                    } else {
                        spend.settle(id, None);
                    }
                })
                .await;
                if let Some(agent) = &agent {
                    let outcome = if refused {
                        AgentSubmitOutcome::Refused
                    } else {
                        AgentSubmitOutcome::Unknown
                    };
                    (agent.answered)(outcome).await;
                }
                let state = if error == PortError::Cancelled {
                    RunState::Canceled
                } else {
                    RunState::Failed(message(&error, kind))
                };
                return self.finish(id, state);
            }
        };
        if let Some(agent) = &agent {
            (agent.answered)(AgentSubmitOutcome::Accepted).await;
        }
        self.log_jobs(jobs, id, &draft, &request, provider.as_ref()).await;
        // Cancelled while the answer was on its way: the jobs it started are cancelled in turn.
        if token.is_cancelled() {
            self.locked().canceling.insert(id);
        }
        self.follow(id, provider);
    }

    /// Writes the jobs a submit started to the job log, so they are followed again after a relaunch.
    async fn log_jobs(
        &self,
        jobs: Vec<RemoteJob>,
        id: Uuid,
        draft: &ImageRequestDraft,
        request: &ImageRequest,
        provider: &dyn ImageProvider,
    ) {
        let date = OffsetDateTime::now_utc();
        let records: Vec<ImageJobRecord> = jobs
            .into_iter()
            .map(|job| ImageJobRecord {
                job,
                owner: provider.owner(),
                run_id: id,
                created_at: date,
                prompt: draft.prompt.clone(),
                model: draft.model.title.clone(),
                aspect_ratio: Some(draft.aspect_ratio.clone()),
                count: draft.count,
                state: ImageJobState::Pending {},
                provider_kind: provider.kind(),
                title: draft.title.clone(),
                request: Some(request.clone()),
                input_images: draft.inputs.clone(),
            })
            .collect();
        let log = self.job_log.clone();
        blocking(move || log.add(records)).await;
    }

    /// Follows the requests of `provider`'s connection that were still running, as after a relaunch, a new sign-in
    /// or a poll that couldn't be read.
    pub fn resume(self: &Arc<Self>, provider: &Arc<dyn ImageProvider>) {
        let pending = self.job_log.pending(provider.owner().as_deref());
        let mut runs: Vec<Uuid> = Vec::new();
        for record in &pending {
            if !runs.contains(&record.run_id) {
                runs.push(record.run_id);
            }
        }
        for run in runs {
            {
                let mut state = self.locked();
                if state.followers.contains_key(&run) || state.submits.contains_key(&run) {
                    continue;
                }
                let Some(first) = pending.iter().find(|record| record.run_id == run) else {
                    continue;
                };
                if let Some(existing) = state.runs.iter_mut().find(|existing| existing.id == run) {
                    existing.state = RunState::Running;
                } else {
                    state.runs.insert(
                        0,
                        ImageRun::new(run, first.model.clone(), first.count, RunState::Running),
                    );
                }
            }
            self.follow(run, provider.clone());
        }
        self.changed();
    }

    /// A new connection: the old one's upload and price are dropped, so nothing of one account reaches another, and
    /// its requests stop being followed until that account is connected again (`resume`). Submits already sent are
    /// left to be answered.
    pub fn reset(&self) {
        let followers: Vec<(Uuid, CancellationToken)> = {
            let mut state = self.locked();
            state.generation += 1;
            state.uploaded = None;
            state.price = None;
            state.price_failure = None;
            state.followers.drain().collect()
        };
        for (run, token) in followers {
            token.cancel();
            self.finish(run, RunState::Interrupted(WAITS_FOR_ACCOUNT.to_owned()));
        }
        self.changed();
    }

    fn follow(self: &Arc<Self>, run: Uuid, provider: Arc<dyn ImageProvider>) {
        let token = CancellationToken::new();
        self.locked().followers.insert(run, token.clone());
        let generator = self.clone();
        tokio::spawn(async move { generator.poll(run, provider, token).await });
    }

    /// Polls until every job of the run ended, backing off like the engine. Errors that waiting can fix are polled
    /// through. Any other leaves the jobs pending and the price counted.
    async fn poll(self: &Arc<Self>, run: Uuid, provider: Arc<dyn ImageProvider>, token: CancellationToken) {
        let mut delay = Backoff::polling();
        let mut interruption: Option<PortError> = None;
        while !token.is_cancelled() && !self.locked().canceling.contains(&run) {
            match self.poll_once(run, provider.as_ref(), &token).await {
                PollRound::Read => {}
                PollRound::Stopped => break,
                PollRound::Interrupted(error) => {
                    interruption = Some(error);
                    break;
                }
            }
            let pending = self
                .job_log
                .records()
                .iter()
                .any(|record| record.run_id == run && record.state == ImageJobState::Pending {});
            if !pending {
                break;
            }
            tokio::select! {
                () = (self.sleep)(delay.next_wait().unsigned_abs()) => {}
                () = token.cancelled() => break,
            }
        }
        self.stop_polling(run, interruption, provider, &token).await;
    }

    async fn poll_once(
        &self,
        run: Uuid,
        provider: &dyn ImageProvider,
        token: &CancellationToken,
    ) -> PollRound {
        let records: Vec<ImageJobRecord> = self
            .job_log
            .records()
            .into_iter()
            .filter(|record| record.run_id == run && record.state == ImageJobState::Pending {})
            .collect();
        for record in records {
            let status = match provider.image_status(&record.job).await {
                Ok(status) => status,
                Err(error) if is_transient(&error) => continue,
                Err(PortError::Cancelled) => return PollRound::Stopped,
                Err(error) => return PollRound::Interrupted(error),
            };
            // A poll answered after the run stopped being followed is read again by whoever follows it next.
            if token.is_cancelled() {
                return PollRound::Stopped;
            }
            self.apply(status, &record).await;
        }
        PollRound::Read
    }

    async fn stop_polling(
        self: &Arc<Self>,
        run: Uuid,
        interruption: Option<PortError>,
        provider: Arc<dyn ImageProvider>,
        token: &CancellationToken,
    ) {
        let canceled = {
            let mut state = self.locked();
            let canceled = state.canceling.remove(&run);
            if canceled {
                state.followers.remove(&run);
            }
            canceled
        };
        if canceled {
            self.cancel_jobs(run, provider.as_ref()).await;
            return;
        }
        // Stopped for another connection: `resume` follows it again.
        if token.is_cancelled() {
            return;
        }
        self.locked().followers.remove(&run);
        if let Some(error) = interruption {
            if error == PortError::Provider(ProviderError::Unauthorized) {
                let handler = self
                    .on_sign_in_expired
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let Some(handler) = handler {
                    handler();
                }
            }
            return self.finish(run, RunState::Interrupted(interrupted(&error, provider.kind())));
        }
        self.end(run).await;
    }

    async fn apply(&self, status: ImageJobStatus, record: &ImageJobRecord) {
        let new_state = match &status {
            ImageJobStatus::Pending => return,
            ImageJobStatus::Completed(_) => ImageJobState::Completed {},
            ImageJobStatus::Failed { message } => ImageJobState::Failed {
                message: message.clone(),
            },
            ImageJobStatus::Rejected(_) => ImageJobState::Rejected {},
            ImageJobStatus::Canceled => ImageJobState::Canceled {},
        };
        let log = self.job_log.clone();
        let id = record.id().to_owned();
        blocking(move || log.update(&id, new_state)).await;
        let ImageJobStatus::Completed(urls) = status else {
            return;
        };
        let parameters = record
            .request
            .as_ref()
            .and_then(|request| serde_json::to_vec(request).ok());
        let new: Vec<GeneratedImage> = urls
            .into_iter()
            .zip(0_i64..)
            .map(|(url, output)| {
                let mut image =
                    GeneratedImage::new(url, &record.prompt, &record.model, record.aspect_ratio.clone());
                image.remote = Some(MaterialRemote {
                    provider: record.provider_kind,
                    owner: record.owner.clone(),
                    id: record.id().to_owned(),
                    output,
                });
                image.parameters.clone_from(&parameters);
                image.input_images.clone_from(&record.input_images);
                image.variant_group_id = Some(record.run_id);
                image.title.clone_from(&record.title);
                image
            })
            .collect();
        {
            let mut state = self.locked();
            let mut images = new.clone();
            images.append(&mut state.images);
            state.images = images;
        }
        self.changed();
        let handler = self
            .on_images
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(handler) = handler {
            handler(new);
        }
    }

    /// Every job of the run ended: charged when one made images, else taken off.
    async fn end(&self, run: Uuid) {
        let records: Vec<ImageJobRecord> = self
            .job_log
            .records()
            .into_iter()
            .filter(|record| record.run_id == run)
            .collect();
        let made = records
            .iter()
            .any(|record| record.state == ImageJobState::Completed {});
        let spend = self.spend.clone();
        blocking(move || {
            if made {
                spend.settle(run, None);
            } else {
                spend.remove(run);
            }
        })
        .await;
        let state = if made {
            RunState::Finished
        } else {
            failure(&records).map_or(RunState::Canceled, RunState::Failed)
        };
        // Ended before the run is forgotten, so `on_run_ended` still reads its jobs.
        self.finish(run, state);
        let log = self.job_log.clone();
        blocking(move || log.forget(run)).await;
    }

    /// A request cancelled while queued isn't charged; one the provider can't cancel any more is, as is one that
    /// already made images.
    async fn cancel_jobs(&self, run: Uuid, provider: &dyn ImageProvider) {
        let mut refused = false;
        let pending: Vec<ImageJobRecord> = self
            .job_log
            .records()
            .into_iter()
            .filter(|record| record.run_id == run && record.state == ImageJobState::Pending {})
            .collect();
        for record in pending {
            if provider.cancel_images(&record.job).await.is_ok() {
                let log = self.job_log.clone();
                let id = record.id().to_owned();
                blocking(move || log.update(&id, ImageJobState::Canceled {})).await;
            } else {
                refused = true;
            }
        }
        let made = self
            .job_log
            .records()
            .iter()
            .any(|record| record.run_id == run && record.state == ImageJobState::Completed {});
        let spend = self.spend.clone();
        blocking(move || {
            if refused || made {
                spend.settle(run, None);
            } else {
                spend.remove(run);
            }
        })
        .await;
        // Each job cancelled is marked so before the run ends, so `on_run_ended` sees which ones went unbilled.
        self.finish(
            run,
            if made {
                RunState::Finished
            } else {
                RunState::Canceled
            },
        );
        let log = self.job_log.clone();
        blocking(move || log.forget(run)).await;
    }

    /// Stops following the run; a request still queued is cancelled and not charged, and an agent's request held
    /// before it was sent is dropped.
    pub fn cancel(&self, id: Uuid) {
        let held = {
            let mut state = self.locked();
            // Held by the guard, its submit sent nothing and ends without it.
            if state
                .runs
                .iter()
                .any(|run| run.id == id && matches!(run.state, RunState::Held(_)))
            {
                true
            } else if let Some(token) = state.submits.get(&id) {
                token.cancel();
                false
            } else if let Some(token) = state.followers.get(&id).cloned() {
                state.canceling.insert(id);
                token.cancel();
                false
            } else {
                false
            }
        };
        if held {
            self.finish(id, RunState::Canceled);
        }
    }

    /// Whether a run with that ID is shown.
    pub fn has_run(&self, id: Uuid) -> bool {
        self.locked().runs.iter().any(|run| run.id == id)
    }

    /// Tells `on_run_ended` a run ended that nothing runs any more (a held request of an earlier launch).
    pub fn report_ended(&self, id: Uuid, state: &RunState) {
        let handler = self
            .on_run_ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(handler) = handler {
            handler(id, state);
        }
    }

    /// Waits for the submits already sent to be answered, so the jobs they start are in the job log before a quit.
    pub async fn finish_submits(&self) {
        loop {
            let notified = self.submitted.notified();
            if self.locked().submits.is_empty() {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn finish(&self, id: Uuid, new: RunState) {
        let ended = (new != RunState::Running).then(|| new.clone());
        if let Some(run) = self.locked().runs.iter_mut().find(|run| run.id == id) {
            run.state = new;
        }
        self.changed();
        if let Some(ended) = ended {
            self.report_ended(id, &ended);
        }
    }

    // MARK: Results

    /// Where the image was saved.
    pub fn set_file(&self, id: Uuid, file: PathBuf) {
        if let Some(image) = self.locked().images.iter_mut().find(|image| image.id == id) {
            image.file = Some(file);
        }
        self.changed();
    }

    /// Takes a result off the list and out of the picks; returns it, `None` when it isn't there.
    pub fn remove(&self, id: Uuid) -> Option<GeneratedImage> {
        let removed = {
            let mut state = self.locked();
            let index = state.images.iter().position(|image| image.id == id)?;
            state.selection.retain(|picked| *picked != id);
            Some(state.images.remove(index))
        };
        self.changed();
        removed
    }

    pub fn contains(&self, id: Uuid) -> bool {
        self.locked().images.iter().any(|image| image.id == id)
    }

    /// The images picked to become references, in the order they were picked.
    pub fn selection(&self) -> Vec<Uuid> {
        self.locked().selection.clone()
    }

    pub fn selected_images(&self) -> Vec<GeneratedImage> {
        let state = self.locked();
        state
            .selection
            .iter()
            .filter_map(|id| state.images.iter().find(|image| image.id == *id).cloned())
            .collect()
    }

    /// Picks `id`, or unpicks it when it is picked. A pick is refused once `room` images are picked (`room` is how many
    /// references Create can still take) and for an image already added; returns whether the selection changed.
    pub fn toggle_selection(&self, id: Uuid, room: usize) -> bool {
        {
            let mut state = self.locked();
            if let Some(index) = state.selection.iter().position(|picked| *picked == id) {
                state.selection.remove(index);
            } else if state.selection.len() < room
                && state
                    .images
                    .iter()
                    .any(|image| image.id == id && !image.is_picked)
            {
                state.selection.push(id);
            } else {
                return false;
            }
        }
        self.changed();
        true
    }

    /// The image was added to Create's references.
    pub fn mark_picked(&self, id: Uuid) {
        {
            let mut state = self.locked();
            let Some(image) = state.images.iter_mut().find(|image| image.id == id) else {
                return;
            };
            image.is_picked = true;
            state.selection.retain(|picked| *picked != id);
        }
        self.changed();
    }

    /// The References quick action was closed: its picks go, the images stay for the Image side of Create.
    pub fn clear_selection(&self) {
        self.locked().selection.clear();
        self.changed();
    }

    /// Drops the images, picks and finished requests; a request still running keeps its row, and its images join the next
    /// results.
    pub fn clear_results(&self) {
        {
            let mut state = self.locked();
            state.images.clear();
            state.selection.clear();
            state.runs.retain(ImageRun::keeps_row);
        }
        self.changed();
    }
}

fn failure(records: &[ImageJobRecord]) -> Option<String> {
    records.iter().find_map(|record| match &record.state {
        ImageJobState::Rejected {} => Some(REFUSED_BY_CHECK.to_owned()),
        ImageJobState::Failed { message } => {
            Some(message.clone().unwrap_or_else(|| COULD_NOT_MAKE.to_owned()))
        }
        _ => None,
    })
}

#[cfg(test)]
#[path = "generator_tests.rs"]
mod tests;
