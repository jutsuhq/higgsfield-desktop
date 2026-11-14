//! Queue, slots, polling, downloads and resume on launch. Talks to storage, media and the network only through the
//! ports in `jutsu-core`.
//!
//! The Swift `actor JobEngine` is a cheap handle over one shared state behind a `std::sync::Mutex` that is held only
//! for sections that don't wait, as an actor's turn between two suspensions. Each change of a job is saved through
//! `apply` or `save`, which take turns per job (`WriteTurns`), so a change made while the job's task runs starts from
//! the task's last change and can't undo it.

mod agent_guard;
mod download;
mod enqueue;
mod import;
mod inputs;
mod leftovers;
mod pipeline;
mod recover;
mod refused_inputs;
mod state;
mod status_batch;
mod variants;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt;
use futures::future::{BoxFuture, Shared};
use jutsu_core::{
    AgentMaxCostRaise, AgentMoney, BudgetError, Connectivity, GenerationProvider, Job, JobEvent,
    JobRepository, JobSpec, JobState, MediaPreparing, OutputNamePattern, PauseReason, PreparedFile,
    SpendStatus, UploadCache,
};
use time::{Duration, OffsetDateTime};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

pub use agent_guard::AgentClearance;
#[cfg(test)]
pub(crate) use download::RESULT_LIFETIME;
pub use enqueue::EnqueueOptions;
#[cfg(test)]
pub(crate) use inputs::upload_cache_key;
pub use leftovers::ScratchFile;
#[cfg(test)]
pub(crate) use leftovers::partial_file;

use crate::agent_spend_file::AgentSpendPolicy;
use crate::bookkeeping::{NetworkGate, ProgressCoalescer, StatusBatch, SubmitTracker, WriteTurns};
use crate::cancel;
use crate::clock::{EngineClock, Random, SystemClock, system_random};
use crate::downloader::{Downloader, HttpDownloader};
use crate::error::EngineError;
use crate::settings::EngineSettings;
use crate::slot_limiter::SlotLimiter;

/// Today's spend without the given job, and the cap of its connection; checked on enqueue and when the provider
/// quotes a price right before submit. `None` skips the check.
pub type SpendStatusFn =
    Arc<dyn Fn(Job) -> BoxFuture<'static, Result<Option<SpendStatus>, BudgetError>> + Send + Sync>;

/// A preset's name for `{preset}` in output names, `None` when unknown. Called while a result is placed, so it must
/// not block.
pub type PresetName = Arc<dyn Fn(Uuid) -> Option<String> + Send + Sync>;

/// What the engine runs on.
#[derive(Clone)]
pub struct EngineDeps {
    /// `None` until the user connects; jobs then wait as after `set_provider(None)`.
    pub provider: Option<Arc<dyn GenerationProvider>>,
    /// Where jobs are saved.
    pub repository: Arc<dyn JobRepository>,
    /// Uploads remembered by content.
    pub upload_cache: Arc<dyn UploadCache>,
    /// Preparing inputs and adding sound to results.
    pub media: Arc<dyn MediaPreparing>,
    /// Whether the computer is online.
    pub connectivity: Arc<dyn Connectivity>,
    /// `None` uses `HttpDownloader`, capped at `settings.max_download_bytes`.
    pub downloader: Option<Arc<dyn Downloader>>,
    /// Time.
    pub clock: Arc<dyn EngineClock>,
    /// The jitter of waits.
    pub random: Random,
    /// Preset names for output names.
    pub preset_name: PresetName,
    /// Today's spend and the cap; `None` skips the check.
    pub spend_status: Option<SpendStatusFn>,
    /// The agent spend ledger and daily budgets the spending guard of agents' jobs checks (`clear_agent_spend`);
    /// `None` sends nothing an agent queued: its jobs pause at submit (`AgentHold::LedgerUnavailable`).
    pub agent_policy: Option<AgentSpendPolicy>,
}

impl EngineDeps {
    /// The required ports, with the system's clock and randomness, no provider, no spend check and no agents.
    pub fn new(
        repository: Arc<dyn JobRepository>,
        upload_cache: Arc<dyn UploadCache>,
        media: Arc<dyn MediaPreparing>,
        connectivity: Arc<dyn Connectivity>,
    ) -> Self {
        Self {
            provider: None,
            repository,
            upload_cache,
            media,
            connectivity,
            downloader: None,
            clock: Arc::new(SystemClock),
            random: system_random(),
            preset_name: Arc::new(|_| None),
            spend_status: None,
            agent_policy: None,
        }
    }
}

/// What the engine tells the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// The job as just saved.
    JobChanged(Box<Job>),
    /// Progress of a job's current step (preparing, uploading, downloading), 0…1; not saved.
    Progress {
        /// The job.
        job_id: Uuid,
        /// The step.
        state: JobState,
        /// How far.
        fraction: f64,
    },
}

/// Runs jobs from enqueue to a downloaded file. Each active job has one task driving it through `JobState`; every
/// transition is saved to the repository before the next step starts, so a crash at any point resumes from the last
/// saved state (`start`). Cloning gives another handle to the same engine.
#[derive(Clone)]
pub struct JobEngine(Arc<Inner>);

pub(crate) struct Inner {
    repository: Arc<dyn JobRepository>,
    upload_cache: Arc<dyn UploadCache>,
    media: Arc<dyn MediaPreparing>,
    connectivity: Arc<dyn Connectivity>,
    downloader: Arc<dyn Downloader>,
    clock: Arc<dyn EngineClock>,
    random: Random,
    preset_name: PresetName,
    spend_status: Option<SpendStatusFn>,
    agent_policy: Option<AgentSpendPolicy>,
    limiter: SlotLimiter,
    events: broadcast::Sender<EngineEvent>,
    state: Mutex<State>,
}

type ConnectionHandler = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;
type SharedJob = Shared<BoxFuture<'static, Result<Job, EngineError>>>;

#[derive(Default)]
pub(crate) struct State {
    provider: Option<Arc<dyn GenerationProvider>>,
    settings: Option<EngineSettings>,
    jobs: HashMap<Uuid, Job>,
    runners: HashMap<Uuid, Runner>,
    prepared: HashMap<Uuid, PreparedInputs>,
    video_urls: HashMap<Uuid, Url>,
    slot_holders: HashSet<Uuid>,
    is_online: bool,
    /// Set on 401: network work waits for `set_provider`.
    connection_needs_attention: bool,
    /// Told when a 401 sets `connection_needs_attention`, so the app asks for the sign-in.
    connection_attention_handler: Option<ConnectionHandler>,
    gate: NetworkGate,
    connectivity_task: Option<JoinHandle<()>>,
    /// Jobs with a change being saved, and the writers waiting for their turn.
    write_turns: WriteTurns,
    is_started: bool,
    /// Jobs `import_finished` is downloading, which a second import of the same job waits for; their partial files
    /// aren't leftovers.
    importing: HashMap<Uuid, SharedJob>,
    progress_coalescer: ProgressCoalescer,
    /// Enqueues not finished yet, by request, so an equal one sent meanwhile gets the same job.
    enqueuing: Vec<(Uuid, enqueue::EnqueueRequest, SharedJob)>,
    /// The enqueue started last; the next one waits for it.
    last_enqueue: Option<Shared<BoxFuture<'static, ()>>>,
    /// Jobs whose submit is being sent or waits for its answer, and the `stop` waiting for them.
    submits: SubmitTracker,
    /// Polled jobs waiting for the next shared status call, and the task that makes it.
    status_batch: StatusBatch,
}

struct Runner {
    token: Uuid,
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PreparedInputs {
    /// `None` for a clip made without a source video.
    video: Option<PreparedFile>,
    images: Vec<PreparedFile>,
    /// The speech (`JobSpec::speech`); `None` without one.
    audio: Option<PreparedFile>,
}

impl JobEngine {
    /// An engine over `deps`, stopped until `start`.
    pub fn new(deps: EngineDeps, settings: EngineSettings) -> Self {
        let downloader = deps
            .downloader
            .unwrap_or_else(|| Arc::new(HttpDownloader::new(settings.max_download_bytes)));
        let (events, _) = broadcast::channel(4096);
        let limiter = SlotLimiter::new(settings.max_concurrency);
        let state = State {
            provider: deps.provider,
            settings: Some(settings),
            is_online: true,
            ..State::default()
        };
        Self(Arc::new(Inner {
            repository: deps.repository,
            upload_cache: deps.upload_cache,
            media: deps.media,
            connectivity: deps.connectivity,
            downloader,
            clock: deps.clock,
            random: deps.random,
            preset_name: deps.preset_name,
            spend_status: deps.spend_status,
            agent_policy: deps.agent_policy,
            limiter,
            events,
            state: Mutex::new(state),
        }))
    }

    /// Every change of every job, for the UI, from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.0.events.subscribe()
    }

    /// The slots.
    pub fn limiter(&self) -> &SlotLimiter {
        &self.0.limiter
    }

    pub(crate) fn st(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn settings(&self) -> EngineSettings {
        self.st()
            .settings
            .clone()
            .unwrap_or_else(|| EngineSettings::new(std::env::temp_dir()))
    }

    pub(crate) fn now(&self) -> OffsetDateTime {
        self.0.clock.now()
    }

    /// The job as the engine holds it now.
    pub(crate) fn job(&self, id: Uuid) -> Option<Job> {
        self.st().jobs.get(&id).cloned()
    }

    /// Call on launch: follows connectivity and resumes every active job from the repository. Jobs with a request
    /// ID are polled, uploaded but unsubmitted jobs are submitted with the same idempotency key, unless a submit
    /// without one may have reached the server; the rest restart their pipeline. Paused jobs stay paused. After
    /// `stop` it resumes the same way. Before that it deletes leftovers of jobs no longer active and forgets uploads
    /// too old to reuse. If the jobs can't be loaded it fails and stays stopped, so it can be called again.
    pub async fn start(&self) -> Result<(), EngineError> {
        {
            let mut state = self.st();
            if state.is_started {
                return Ok(());
            }
            state.is_started = true;
        }
        self.follow_connectivity().await;
        let loaded = async {
            let active = self.0.repository.active_jobs().await?;
            let ids = self.0.repository.active_job_ids().await?;
            Ok::<_, jutsu_core::PortError>((active, ids))
        }
        .await;
        let (active_jobs, stored_ids) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                let mut state = self.st();
                if let Some(task) = state.connectivity_task.take() {
                    task.abort();
                }
                state.is_started = false;
                return Err(error.into());
            }
        };
        self.remove_leftovers(&active_jobs, &stored_ids).await;
        self.reconcile_agent_reservations().await;
        for stored in active_jobs {
            let job = {
                let mut state = self.st();
                let job = match state.jobs.get(&stored.id) {
                    Some(known) => known.clone(),
                    None => adopting_legacy_owner(state.provider.as_deref(), stored),
                };
                state.jobs.insert(job.id, job.clone());
                if !is_runnable(&job.state) || state.runners.contains_key(&job.id) {
                    continue;
                }
                job
            };
            self.forget_retries(&job).await;
            // Accepted jobs hold their slots before anything new can take them.
            if matches!(job.state, JobState::Queued | JobState::Processing) {
                self.occupy_slot(job.id);
            }
            self.start_runner(job.id);
        }
        Ok(())
    }

    /// Before submit the job is dropped locally; while queued the API cancels it too. While downloading the
    /// generation is already finished, so only the download stops and its partial file is removed. Fails with
    /// `NotCancellable` while the API is processing it.
    ///
    /// A submit that may have reached Higgsfield is never dropped as canceled: while such a submit waits for its
    /// answer (`is_submit_pending`) this fails with `SubmitInProgress`. A submit sent without a key can't be resent,
    /// so cutting a running one off fails the job as `OutcomeUnknown`.
    pub async fn cancel(&self, id: Uuid) -> Result<(), EngineError> {
        let current = self.load_job(id).await?;
        if !current.state.is_cancellable() {
            return Err(EngineError::NotCancellable(current.state));
        }
        if is_submit_pending(&current) {
            return Err(EngineError::SubmitInProgress);
        }
        self.stop_runner(id).await;
        let job = self.load_job(id).await?;
        if !job.state.is_cancellable() {
            self.start_runner(id);
            return Err(EngineError::NotCancellable(job.state));
        }
        if is_submit_pending(&job) {
            self.start_runner(id);
            return Err(EngineError::SubmitInProgress);
        }
        if job.state == JobState::Queued
            && let Some(remote) = &job.remote
        {
            let cancelled = match self.current_provider(&job) {
                Ok(provider) => provider.cancel(remote).await.map_err(EngineError::from),
                Err(error) => Err(error),
            };
            if let Err(error) = cancelled {
                self.start_runner(id);
                return Err(error);
            }
        }
        let may_have_started = job.submit_attempt.is_some();
        let event = if may_have_started {
            JobEvent::Failed {
                reason: jutsu_core::FailureReason::OutcomeUnknown,
            }
        } else {
            JobEvent::Cancel
        };
        self.apply(event, id, |_| {}).await?;
        // Higgsfield cancelled it while queued, so it isn't billed.
        if job.state == JobState::Queued && job.remote.is_some() && !may_have_started {
            self.unbill_agent_spend(id).await;
        }
        Ok(())
    }

    /// A failed or canceled job whose generation finished downloads that paid result again, and one that failed at
    /// the deadline is checked again (`check_again`). Any other runs again from the start as a new attempt, with a
    /// new idempotency key unless a submit of it may have reached the server. An agent's reservation of the last
    /// attempt keeps counting at its reserved amount; a ledger that can't record that fails, and the job stays as it
    /// is.
    pub async fn retry(&self, id: Uuid) -> Result<(), EngineError> {
        let job = self.load_job(id).await?;
        if !matches!(job.state, JobState::Failed { .. } | JobState::Canceled) {
            return Err(EngineError::NotRetryable(job.state));
        }
        if job.may_still_finish() {
            return self.check_again(id).await;
        }
        let now = self.now();
        if job.has_paid_output() {
            self.apply(JobEvent::RetryDownload, id, move |job| job.retried_at = Some(now))
                .await?;
        } else {
            // The new attempt reserves anew; what the last one may have been charged keeps counting.
            self.keep_agent_spend(job.agent.as_ref().and_then(|agent| agent.reservation), id)
                .await?;
            let new_key = job.submit_attempt.is_none();
            self.apply(JobEvent::Retry, id, move |job| {
                job.retried_at = Some(now);
                job.inputs = None;
                job.remote = None;
                job.output_file = None;
                job.output_media = None;
                job.submit_attempt = None;
                job.server_price = None;
                job.approved_price = None;
                if let Some(agent) = job.agent.as_mut() {
                    agent.start_over();
                }
                job.submitted_at = None;
                job.processing_at = None;
                job.promoted_at = None;
                job.checked_again_at = None;
                if new_key {
                    job.idempotency_key = Uuid::new_v4();
                }
            })
            .await?;
        }
        self.start_runner(id);
        Ok(())
    }

    /// Polls a job that failed at the deadline (`Job::may_still_finish`) again, with a new deadline. It keeps its
    /// request, so a generation that finished meanwhile is downloaded instead of paid for twice.
    pub async fn check_again(&self, id: Uuid) -> Result<(), EngineError> {
        let job = self.load_job(id).await?;
        if !job.may_still_finish() {
            return Err(EngineError::NotRetryable(job.state));
        }
        let now = self.now();
        self.apply(JobEvent::CheckAgain, id, move |job| {
            job.timed_out_at = None;
            job.checked_again_at = Some(now);
        })
        .await?;
        self.start_runner(id);
        Ok(())
    }

    /// Continues a paused job from the start of its pipeline; prepared files and uploads are cached. A job paused for
    /// `NotEnoughDiskSpace` continues its download from the partial file instead. Resuming a job paused for
    /// `PriceAboveCap` agrees to the price the provider quoted, so ask first. Resuming an agent's job paused for
    /// `AgentLimit` is the person agreeing to send it past the agent's limits at its price; only the person does
    /// this, never an agent. A job that went on meanwhile is left as it is.
    pub async fn resume(&self, id: Uuid) -> Result<(), EngineError> {
        let job = self.load_job(id).await?;
        let paused = job.state.clone();
        let still_paused = move |now: &Job| now.state == paused;
        match &job.state {
            JobState::Paused(PauseReason::PriceAboveCap) => {
                self.apply_if_still(JobEvent::Resume, id, still_paused, |job| {
                    job.approved_price = Some(job.price().clone());
                })
                .await?;
            }
            JobState::Paused(PauseReason::AgentLimit) => {
                self.apply_if_still(JobEvent::Resume, id, still_paused, |job| {
                    job.approved_price = Some(job.price().clone());
                    if let Some(agent) = job.agent.as_mut() {
                        agent.approved_by_person = true;
                        agent.hold = None;
                    }
                })
                .await?;
            }
            JobState::Paused(_) => {
                self.apply_if_still(JobEvent::Resume, id, still_paused, |job| {
                    if let Some(agent) = job.agent.as_mut() {
                        agent.paused_by_agent = None;
                    }
                })
                .await?;
            }
            state if !state.is_active() => return Err(EngineError::NotResumable(job.state)),
            _ => {}
        }
        self.start_runner(id);
        Ok(())
    }

    /// Moves a job waiting for a slot ahead of the other waiters, so it takes the next free slot. Saved with the
    /// job, so the order holds after a restart. A later Run next on another job goes ahead of this one.
    pub async fn run_next(&self, id: Uuid) -> Result<(), EngineError> {
        let job = self.load_job(id).await?;
        if job.state != JobState::WaitingForSlot || job.submit_attempt.is_some() {
            return Err(EngineError::NotWaiting(job.state));
        }
        let now = self.now();
        self.save(id, move |job| job.promoted_at = Some(now)).await?;
        self.0.limiter.promote(id, now);
        Ok(())
    }

    /// `handler` runs each time the provider turns the connection down (401) while jobs depend on it.
    pub fn on_connection_needs_attention(
        &self,
        handler: impl Fn() -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) {
        self.st().connection_attention_handler = Some(Arc::new(handler));
    }

    /// Switches the connection. Unsubmitted jobs of another provider or account pause with
    /// `ConnectionNeedsAttention`; submitted ones wait until their connection is back. Jobs paused for the
    /// connection resume when their connection returns.
    pub async fn set_provider(&self, provider: Option<Arc<dyn GenerationProvider>>) {
        Box::pin(self.switch_provider(provider)).await;
    }

    async fn switch_provider(&self, provider: Option<Arc<dyn GenerationProvider>>) {
        let active: Vec<Job> = {
            let mut state = self.st();
            state.provider.clone_from(&provider);
            state.connection_needs_attention = false;
            state
                .jobs
                .values()
                .filter(|job| job.state.is_active())
                .cloned()
                .collect()
        };
        for job in active {
            if provider.as_ref().is_some_and(|provider| provider.owns(&job)) {
                if job.state != JobState::Paused(PauseReason::ConnectionNeedsAttention) {
                    continue;
                }
                if let Err(error) = self.resume(job.id).await {
                    tracing::error!(job = %job.id, %error, "can't resume");
                }
            } else if is_before_submit(&job.state) {
                self.stop_runner(job.id).await;
                if !self.job(job.id).is_some_and(|job| is_before_submit(&job.state)) {
                    continue;
                }
                if let Err(error) = self
                    .apply(
                        JobEvent::Paused(PauseReason::ConnectionNeedsAttention),
                        job.id,
                        |_| {},
                    )
                    .await
                {
                    tracing::error!(job = %job.id, ?error, "can't pause");
                }
            }
        }
        self.st().gate.open();
    }

    async fn follow_connectivity(&self) {
        let mut updates = self.0.connectivity.updates();
        // The first value is the current state; wait for it so nothing starts while offline.
        if let Some(online) = updates.next().await {
            self.st().is_online = online;
        }
        let engine = Arc::downgrade(&self.0);
        let task = tokio::spawn(async move {
            while let Some(online) = updates.next().await {
                let Some(inner) = engine.upgrade() else { break };
                let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.is_online = online;
                state.gate.open();
            }
        });
        self.st().connectivity_task = Some(task);
    }
}

// Pause all and Resume all, the person's and an agent's.
impl JobEngine {
    /// Pauses every job not yet submitted with `ByUser`, so none takes a slot until `resume_all`. Submitted jobs keep
    /// polling and downloading. A job whose submit is already sent is left to finish it. `agents_only` is an agent's
    /// `pause_queue`: it pauses only the jobs agents queued and marks them (`AgentOrigin::paused_by_agent`), so the
    /// agent's `resume_all(true)` lifts only its own pause. The person's Pause all takes over the jobs an agent
    /// paused, so no agent lifts it.
    pub async fn pause_all(&self, agents_only: bool) {
        self.pause(agents_only, agents_only).await;
    }

    /// The person's Pause agents: pauses the jobs agents queued as Pause all pauses every job, so no agent's
    /// `resume_all(true)` lifts it and the person's own jobs run on. `resume_agent_jobs` lifts it.
    pub async fn pause_agent_jobs(&self) {
        self.pause(true, false).await;
    }

    async fn pause(&self, agents_only: bool, by_agent: bool) {
        let pausable = move |job: &Job| {
            job.state.is_pausable_by_user()
                && job.submit_attempt.is_none()
                && (!agents_only || job.agent.is_some())
        };
        let candidates: Vec<Uuid> = self
            .st()
            .jobs
            .values()
            .filter(|job| pausable(job))
            .map(|job| job.id)
            .collect();
        for id in candidates {
            self.stop_runner(id).await;
            if !self.job(id).is_some_and(|job| pausable(&job)) {
                self.start_runner(id);
                continue;
            }
            let paused = self
                .apply(JobEvent::Paused(PauseReason::ByUser), id, move |job| {
                    if by_agent && let Some(agent) = job.agent.as_mut() {
                        agent.paused_by_agent = Some(true);
                    }
                })
                .await;
            if let Err(error) = paused {
                tracing::error!(job = %id, ?error, "can't pause");
                self.start_runner(id);
            }
        }
        if by_agent {
            return;
        }
        // The person's pause takes over the jobs an agent paused, so no agent lifts it.
        let taken_over: Vec<Uuid> = self
            .st()
            .jobs
            .values()
            .filter(|job| {
                job.state == JobState::Paused(PauseReason::ByUser)
                    && job
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.paused_by_agent == Some(true))
            })
            .map(|job| job.id)
            .collect();
        for id in taken_over {
            let saved = self
                .save(id, |job| {
                    if let Some(agent) = job.agent.as_mut() {
                        agent.paused_by_agent = None;
                    }
                })
                .await;
            if let Err(error) = saved {
                tracing::error!(job = %id, ?error, "can't save");
            }
        }
    }

    /// Resumes the jobs `pause_all` paused, oldest first. Other pauses stay. `agents_only`, an agent's
    /// `resume_queue`, resumes only the jobs an agent's `pause_all(true)` paused, never the person's own pause.
    pub async fn resume_all(&self, agents_only: bool) {
        Box::pin(self.resume_where(move |job| {
            job.state == JobState::Paused(PauseReason::ByUser)
                && (!agents_only
                    || job
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.paused_by_agent == Some(true)))
        }))
        .await;
    }

    /// Lifts the person's Pause agents, and any other pause of an agent's job made with Pause all or an agent's own.
    pub async fn resume_agent_jobs(&self) {
        Box::pin(
            self.resume_where(|job| {
                job.state == JobState::Paused(PauseReason::ByUser) && job.agent.is_some()
            }),
        )
        .await;
    }

    async fn resume_where(&self, is_resumed: impl Fn(&Job) -> bool) {
        let mut resumed: Vec<Job> = self
            .st()
            .jobs
            .values()
            .filter(|job| is_resumed(job))
            .cloned()
            .collect();
        resumed.sort_by_key(|job| job.created_at);
        for job in resumed {
            if let Err(error) = self.resume(job.id).await {
                tracing::error!(job = %job.id, %error, "can't resume");
            }
        }
    }
}

// Settings that apply from the next job on.
impl JobEngine {
    /// How many jobs may be submitted or running at once.
    pub fn set_max_concurrency(&self, limit: usize) {
        self.0.limiter.set_limit(limit);
    }

    /// Downloads that start after this call land in `directory`; finished files stay where they are.
    pub fn set_output_directory(&self, directory: PathBuf) {
        if let Some(settings) = self.st().settings.as_mut() {
            settings.output_directory = directory;
        }
    }

    /// Results placed after this call use `pattern`; files already in the library keep their names.
    pub fn set_output_name_pattern(&self, pattern: OutputNamePattern) {
        if let Some(settings) = self.st().settings.as_mut() {
            settings.output_name_pattern = pattern;
        }
    }
}

// Stopping, as on quit.
impl JobEngine {
    /// Stops all work without changing any job, as if the app quit; `start` picks it up again.
    ///
    /// Submits already sent first get up to `finishing_submits_within` (wall-clock time) to be answered, so quitting
    /// doesn't cut off a request Higgsfield may already be running; no new submit is sent meanwhile. A submit still
    /// unanswered after that is resent with its idempotency key on the next start, or fails there as
    /// `FailureReason::SubmitCutOff` when its provider can't send one.
    pub async fn stop(&self, finishing_submits_within: Duration) {
        self.st().submits.is_stopping = true;
        self.wait_for_submits(finishing_submits_within).await;
        let (runners, ticker) = {
            let mut state = self.st();
            if let Some(task) = state.connectivity_task.take() {
                task.abort();
            }
            state.is_started = false;
            (
                std::mem::take(&mut state.runners),
                state.status_batch.ticker.take(),
            )
        };
        for runner in runners.values() {
            runner.cancel.cancel();
        }
        if let Some(ticker) = ticker {
            ticker.cancel();
        }
        for runner in runners.into_values() {
            let _ = runner.handle.await;
        }
        self.st().submits.is_stopping = false;
    }

    /// Returns once no submit is out, or after `grace`.
    async fn wait_for_submits(&self, grace: Duration) {
        let (sender, released) = oneshot::channel();
        {
            let mut state = self.st();
            if state.submits.in_flight.is_empty() || grace <= Duration::ZERO {
                return;
            }
            state.submits.wait(sender);
        }
        let engine = self.clone();
        let wait = std::time::Duration::try_from(grace).unwrap_or(std::time::Duration::MAX);
        let timer = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let mut state = engine.st();
            let unanswered = state.submits.in_flight.len();
            if state.submits.release_waiters() {
                tracing::error!(unanswered, "stopping with submits unanswered");
            }
        });
        let _ = released.await;
        timer.abort();
    }
}

// Loading jobs and their runners.
impl JobEngine {
    pub(crate) async fn load_job(&self, id: Uuid) -> Result<Job, EngineError> {
        if let Some(job) = self.job(id) {
            return Ok(job);
        }
        let Some(stored) = self.0.repository.job(id).await? else {
            return Err(EngineError::UnknownJob(id));
        };
        let mut state = self.st();
        let job = match state.jobs.get(&id) {
            Some(known) => known.clone(),
            None => adopting_legacy_owner(state.provider.as_deref(), stored),
        };
        state.jobs.insert(id, job.clone());
        Ok(job)
    }

    /// A runner counts failures from its own start: a wait from before a quit says nothing about now.
    async fn forget_retries(&self, job: &Job) {
        if job.retrying.is_none() {
            return;
        }
        if let Err(error) = self.save(job.id, |job| job.retrying = None).await {
            tracing::error!(job = %job.id, ?error, "can't save");
        }
    }

    pub(crate) fn start_runner(&self, id: Uuid) {
        let mut state = self.st();
        if state.runners.contains_key(&id) || !state.jobs.get(&id).is_some_and(|job| is_runnable(&job.state))
        {
            return;
        }
        let token = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let engine = self.clone();
        let drive: BoxFuture<'static, ()> = Box::pin(async move { Box::pin(engine.drive(id, token)).await });
        let handle = tokio::spawn(cancel::scope(cancel.clone(), drive));
        state.runners.insert(
            id,
            Runner {
                token,
                cancel,
                handle,
            },
        );
    }

    pub(crate) async fn stop_runner(&self, id: Uuid) {
        let Some(runner) = self.st().runners.remove(&id) else {
            return;
        };
        runner.cancel.cancel();
        let _ = runner.handle.await;
    }

    /// The partial file of the job's download: in the folder the download started in, or the output folder before
    /// one starts.
    pub(crate) fn partial_download(&self, id: Uuid) -> PathBuf {
        let folder = self
            .job(id)
            .and_then(|job| job.download_folder)
            .and_then(|folder| folder.to_file_path().ok())
            .unwrap_or_else(|| self.settings().output_directory);
        leftovers::partial_file(id, &folder)
    }

    /// Whether a task drives the job now.
    #[cfg(test)]
    pub(crate) fn has_runner(&self, id: Uuid) -> bool {
        self.st().runners.contains_key(&id)
    }

    /// How many jobs a task drives now.
    #[cfg(test)]
    pub(crate) fn runner_count(&self) -> usize {
        self.st().runners.len()
    }

    /// How many `stop`s wait for submits to be answered.
    #[cfg(test)]
    pub(crate) fn submit_waiter_count(&self) -> usize {
        self.st().submits.waiter_count()
    }

    /// Whether a change of the job waits for its write turn.
    #[cfg(test)]
    pub(crate) fn has_write_waiters(&self, id: Uuid) -> bool {
        self.st().write_turns.has_waiters(id)
    }

    /// Changes the agent's `max_cost` on its held job; see `agent_guard`.
    pub async fn raise_agent_max_cost(
        &self,
        id: Uuid,
        max_cost: AgentMoney,
    ) -> Result<AgentMaxCostRaise, EngineError> {
        self.raise_max_cost(id, max_cost).await
    }

    /// Queues a job; see `enqueue`.
    pub async fn enqueue(&self, spec: JobSpec, options: EnqueueOptions) -> Result<Job, EngineError> {
        self.enqueue_job(spec, options).await
    }
}

/// Jobs stored before owners existed belong to the connection active when the engine first loads them, normally the
/// one restored at launch, which queued them.
fn adopting_legacy_owner(provider: Option<&dyn GenerationProvider>, mut job: Job) -> Job {
    if job.owner.is_none()
        && let Some(provider) = provider
        && provider.kind() == job.provider_kind
    {
        job.owner = provider.owner();
    }
    job
}

/// A sent submit that may have reached the server and still waits for its answer: one with the idempotency key,
/// which the job's task resends with that key until answered, or any submit a pause cut off, which waits for the
/// job's connection like a submitted job.
pub(crate) fn is_submit_pending(job: &Job) -> bool {
    job.submit_attempt
        .is_some_and(|attempt| attempt.is_idempotent || !is_runnable(&job.state))
}

pub(crate) fn is_queued_or_processing(state: &JobState) -> bool {
    matches!(state, JobState::Queued | JobState::Processing)
}

pub(crate) fn is_before_submit(state: &JobState) -> bool {
    matches!(
        state,
        JobState::Preparing | JobState::Uploading | JobState::WaitingForSlot
    )
}

/// Active and not waiting for the user.
pub(crate) fn is_runnable(state: &JobState) -> bool {
    !matches!(state, JobState::Paused(_)) && state.is_active()
}

/// A local path as a `file:` URL.
pub(crate) fn file_url(path: &std::path::Path) -> Result<Url, EngineError> {
    Url::from_file_path(path).map_err(|()| EngineError::Io {
        kind: std::io::ErrorKind::InvalidInput,
        message: format!("Not an absolute path: {}", path.display()),
    })
}
