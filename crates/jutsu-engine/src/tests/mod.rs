//! Fakes of every port the engine talks to, and a harness that makes engines over them. No test sleeps for real:
//! the clock is virtual, and waits are on what the repository saves.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::struct_excessive_bools,
    clippy::match_same_arms,
    clippy::enum_variant_names,
    reason = "test code fails loudly"
)]

mod agent;
mod agent_max_cost;
mod disk_space;
mod engine;
mod enqueue;
mod history;
mod ownership;
mod pause_all;
mod post_operation;
mod retry;
mod run_next;
mod server_price;
mod status_batch;
mod unlimited;
mod variants;

mod bookkeeping;
mod downloader;
mod failure_reason_mapping;
mod file_replacement;
mod http_stub;
mod leftovers;
mod linked_inputs;
mod media_link_downloader;
mod progress_coalescing;
mod server_image_reader;
mod server_media_inputs;
mod server_url;
mod slot_limiter;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use jutsu_core::{
    Balance, Capabilities, ClosedRange, Connectivity, Cost, Job, JobRepository, JobSpec, JobState,
    LocalAudio, LocalImage, MediaLinkKind, MediaPreparing, Mode, OutputMedia, PixelSize, PortError,
    PreparedFile, ProviderError, ProviderKind, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus,
    Resolution, SourceClip, SpendStatus, StylePreset, UploadCache,
};
use jutsu_core::{FailureReason, GenerationProvider, PauseReason, Progress};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

use crate::clock::EngineClock;
use crate::downloader::{DownloadError, Downloader};
use crate::{AgentSpendPolicy, EngineDeps, EngineSettings, JobEngine};

/// Calls in the order the fakes saw them.
#[derive(Clone, Default)]
pub(crate) struct CallLog(Arc<Mutex<Vec<String>>>);

impl CallLog {
    pub(crate) fn push(&self, entry: impl Into<String>) {
        self.0.lock().unwrap().push(entry.into());
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub(crate) fn count(&self, entry: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == entry)
            .count()
    }
}

/// A one-shot latch a fake waits on until the test opens it.
#[derive(Clone)]
pub(crate) struct Gate(watch::Sender<bool>);

impl Gate {
    pub(crate) fn new() -> Self {
        Self(watch::Sender::new(false))
    }

    pub(crate) fn open(&self) {
        self.0.send_replace(true);
    }

    pub(crate) async fn wait(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|open| *open).await;
    }

    /// Someone waits for the gate to open.
    pub(crate) fn is_waiting(&self) -> bool {
        self.0.receiver_count() > 0
    }
}

/// A virtual clock: a sleep records its length, moves the time on and lets other tasks run.
pub(crate) struct TestClock {
    state: Mutex<(OffsetDateTime, Vec<Duration>)>,
}

impl TestClock {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new((
                OffsetDateTime::from_unix_timestamp(1_791_892_800).unwrap(),
                Vec::new(),
            )),
        }
    }

    pub(crate) fn sleeps(&self) -> Vec<Duration> {
        self.state.lock().unwrap().1.clone()
    }

    pub(crate) fn advance(&self, by: Duration) {
        self.state.lock().unwrap().0 += by;
    }
}

#[async_trait]
impl EngineClock for TestClock {
    fn now(&self) -> OffsetDateTime {
        self.state.lock().unwrap().0
    }

    async fn sleep(&self, duration: Duration) {
        {
            let mut state = self.state.lock().unwrap();
            state.1.push(duration);
            state.0 += duration;
        }
        tokio::task::yield_now().await;
    }
}

/// The name a state is logged by, as Swift's description of it.
pub(crate) fn state_name(state: &JobState) -> String {
    match state {
        JobState::Preparing => "preparing".into(),
        JobState::Uploading => "uploading".into(),
        JobState::WaitingForSlot => "waitingForSlot".into(),
        JobState::Queued => "queued".into(),
        JobState::Processing => "processing".into(),
        JobState::Downloading => "downloading".into(),
        JobState::Done => "done".into(),
        JobState::Failed { .. } => "failed".into(),
        JobState::Rejected => "rejected".into(),
        JobState::Canceled => "canceled".into(),
        JobState::Paused(_) => "paused".into(),
    }
}

#[derive(Default)]
struct Stored {
    jobs: HashMap<Uuid, Job>,
    history: HashMap<Uuid, Vec<JobState>>,
    fail_next_load: bool,
    held_save: Option<HeldSave>,
}

/// A save that is stored, then doesn't return until `release` opens, like a slow disk.
struct HeldSave {
    matches: Box<dyn Fn(&Job) -> bool + Send>,
    reached: Gate,
    release: Gate,
}

/// Jobs in memory, with every state each one was saved in.
#[derive(Clone)]
pub(crate) struct InMemoryRepository {
    log: CallLog,
    stored: Arc<Mutex<Stored>>,
    version: watch::Sender<u64>,
    /// IDs that are active but can't be decoded.
    pub(crate) undecodable: Arc<Mutex<HashSet<Uuid>>>,
}

impl InMemoryRepository {
    pub(crate) fn new(log: CallLog) -> Self {
        Self {
            log,
            stored: Arc::default(),
            version: watch::Sender::new(0),
            undecodable: Arc::default(),
        }
    }

    pub(crate) fn stored(&self, id: Uuid) -> Option<Job> {
        self.stored.lock().unwrap().jobs.get(&id).cloned()
    }

    pub(crate) fn all(&self) -> Vec<Job> {
        self.stored.lock().unwrap().jobs.values().cloned().collect()
    }

    pub(crate) fn history(&self, id: Uuid) -> Vec<JobState> {
        self.stored
            .lock()
            .unwrap()
            .history
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn put(&self, job: Job) {
        self.stored.lock().unwrap().jobs.insert(job.id, job);
    }

    pub(crate) fn fail_next_load(&self) {
        self.stored.lock().unwrap().fail_next_load = true;
    }

    /// Waits until job `id` as stored matches `predicate`; fails the test after 10 s.
    pub(crate) async fn wait(&self, id: Uuid, predicate: impl Fn(&Job) -> bool) -> Job {
        let mut changes = self.version.subscribe();
        let waited = tokio::time::timeout(StdDuration::from_secs(10), async {
            loop {
                if let Some(job) = self.stored(id).filter(|job| predicate(job)) {
                    return job;
                }
                changes.changed().await.unwrap();
            }
        })
        .await;
        waited.unwrap_or_else(|_| {
            panic!(
                "job {id} never matched; stored: {:?}",
                self.stored(id).map(|job| job.state)
            )
        })
    }

    pub(crate) async fn wait_state(&self, id: Uuid, state: &JobState) -> Job {
        self.wait(id, |job| job.state == *state).await
    }

    /// Waits until job `id` has been saved in `state` at some point; fails the test after 10 s.
    pub(crate) async fn wait_reached(&self, id: Uuid, state: &JobState) -> Job {
        self.wait_until(|repository| repository.history(id).contains(state))
            .await;
        self.stored(id).unwrap()
    }

    /// Waits until `predicate` holds for what has been saved so far; fails the test after 10 s.
    pub(crate) async fn wait_until(&self, predicate: impl Fn(&Self) -> bool) {
        let mut changes = self.version.subscribe();
        let waited = tokio::time::timeout(StdDuration::from_secs(10), async {
            while !predicate(self) {
                changes.changed().await.unwrap();
            }
        })
        .await;
        assert!(waited.is_ok(), "the repository never reached the state");
    }

    /// The next save of a job that `matches` is stored, then doesn't return until the second gate opens; the first
    /// opens once that save is held.
    pub(crate) fn hold_save(&self, matches: impl Fn(&Job) -> bool + Send + 'static) -> (Gate, Gate) {
        let (reached, release) = (Gate::new(), Gate::new());
        self.stored.lock().unwrap().held_save = Some(HeldSave {
            matches: Box::new(matches),
            reached: reached.clone(),
            release: release.clone(),
        });
        (reached, release)
    }
}

#[async_trait]
impl JobRepository for InMemoryRepository {
    async fn save(&self, job: &Job) -> Result<(), PortError> {
        self.log.push(format!("save {}", state_name(&job.state)));
        let held = {
            let mut stored = self.stored.lock().unwrap();
            stored.history.entry(job.id).or_default().push(job.state.clone());
            stored.jobs.insert(job.id, job.clone());
            if stored.held_save.as_ref().is_some_and(|held| (held.matches)(job)) {
                stored.held_save.take()
            } else {
                None
            }
        };
        self.version.send_modify(|version| *version += 1);
        if let Some(held) = held {
            held.reached.open();
            held.release.wait().await;
        }
        Ok(())
    }

    async fn job(&self, id: Uuid) -> Result<Option<Job>, PortError> {
        Ok(self.stored(id))
    }

    async fn active_jobs(&self) -> Result<Vec<Job>, PortError> {
        let mut stored = self.stored.lock().unwrap();
        if std::mem::take(&mut stored.fail_next_load) {
            return Err(PortError::Other("the store can't be read".into()));
        }
        Ok(stored
            .jobs
            .values()
            .filter(|job| job.state.is_active())
            .cloned()
            .collect())
    }

    async fn active_job_ids(&self) -> Result<HashSet<Uuid>, PortError> {
        let stored = self.stored.lock().unwrap();
        let mut ids: HashSet<Uuid> = stored
            .jobs
            .values()
            .filter(|job| job.state.is_active())
            .map(|job| job.id)
            .collect();
        ids.extend(self.undecodable.lock().unwrap().iter());
        Ok(ids)
    }

    async fn delete(&self, id: Uuid) -> Result<(), PortError> {
        self.stored.lock().unwrap().jobs.remove(&id);
        Ok(())
    }
}

/// What a submit does.
pub(crate) enum SubmitStep {
    Fail(PortError),
    /// Accepted by the server, but the answer never comes.
    Hang,
    /// Accepted; answers once the gate opens.
    Answer(Gate),
}

#[derive(Default)]
pub(crate) struct ProviderState {
    pub(crate) submit_steps: VecDeque<SubmitStep>,
    pub(crate) status_steps: VecDeque<Result<RemoteStatus, PortError>>,
    pub(crate) held_status: Option<RemoteStatus>,
    pub(crate) upload_errors: VecDeque<PortError>,
    pub(crate) uploads: Vec<String>,
    pub(crate) accepted: HashMap<Uuid, RemoteJob>,
    pub(crate) submit_keys: Vec<Uuid>,
    pub(crate) submitted_inputs: Vec<RemoteInputs>,
    pub(crate) submit_status: Option<RemoteStatus>,
    pub(crate) unlimited_answer: (bool, Option<Cost>),
    pub(crate) variants_started: Option<i64>,
    pub(crate) quote: Option<Cost>,
    pub(crate) pays_unlimited: bool,
    pub(crate) not_idempotent: bool,
    pub(crate) status_batch_limit: usize,
    pub(crate) cancels: Vec<String>,
    pub(crate) preliminary: Option<Cost>,
    pub(crate) charge: Option<Cost>,
    pub(crate) in_flight: HashSet<String>,
    pub(crate) max_in_flight: usize,
    pub(crate) status_asked: Vec<String>,
    pub(crate) batch_calls: Vec<Vec<String>>,
    pub(crate) linked: bool,
    pub(crate) server_media: bool,
    pub(crate) kind: Option<ProviderKind>,
    pub(crate) owner: Option<String>,
    /// What `server_estimate` fails with, when set (it then logs "serverEstimate" too).
    pub(crate) quote_error: Option<PortError>,
    /// What `charge` fails with, when set.
    pub(crate) charge_error: Option<PortError>,
    pub(crate) batch_mode: BatchMode,
    /// Put into every batch answer.
    pub(crate) batch_poll_after: Option<Duration>,
    /// Request IDs every batch answer leaves out.
    pub(crate) batch_omits: HashSet<String>,
    /// Answers by request ID that override `held_status` in batch answers.
    pub(crate) batch_statuses: HashMap<String, RemoteStatus>,
}

/// What `statuses` does.
#[derive(Default)]
pub(crate) enum BatchMode {
    /// Answers each job with `batch_statuses`, `held_status` or completed, except `batch_omits`.
    #[default]
    Answer,
    /// `None`, like an account without `jobs_wait`.
    Unavailable,
    Fail(PortError),
    /// Answers once the gate opens.
    Held(Gate),
}

/// A provider that answers as the test scripts it.
#[derive(Clone)]
pub(crate) struct FakeProvider {
    pub(crate) log: CallLog,
    pub(crate) state: Arc<Mutex<ProviderState>>,
}

pub(crate) fn video_url() -> Url {
    Url::parse("https://cdn.example/result.mp4").unwrap()
}

pub(crate) fn usd(amount: &str) -> Cost {
    Cost::usd(amount.parse::<Decimal>().unwrap(), false)
}

impl FakeProvider {
    pub(crate) fn new(log: CallLog) -> Self {
        Self {
            log,
            state: Arc::default(),
        }
    }

    pub(crate) fn with<T>(&self, change: impl FnOnce(&mut ProviderState) -> T) -> T {
        change(&mut self.state.lock().unwrap())
    }

    fn accept(&self, key: Uuid, spec: &JobSpec) -> RemoteJob {
        let mut state = self.state.lock().unwrap();
        if let Some(known) = state.accepted.get(&key) {
            return known.clone();
        }
        let mut remote = RemoteJob::new(format!("r{}", state.accepted.len() + 1));
        remote.initial_status.clone_from(&state.submit_status);
        remote.paid_with_unlimited = state.unlimited_answer.0;
        remote.charged.clone_from(&state.unlimited_answer.1);
        let started = state.variants_started.unwrap_or(spec.variants).max(1);
        remote.variants = (1..started)
            .map(|index| RemoteJob::new(format!("{}-v{index}", remote.request_id)))
            .collect();
        state.accepted.insert(key, remote.clone());
        state.in_flight.insert(remote.request_id.clone());
        for variant in &remote.variants {
            state.in_flight.insert(variant.request_id.clone());
        }
        state.max_in_flight = state.max_in_flight.max(state.in_flight.len());
        remote
    }
}

#[async_trait]
impl GenerationProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        self.with(|state| state.kind.unwrap_or(ProviderKind::ApiKey))
    }

    fn owner(&self) -> Option<String> {
        self.with(|state| state.owner.clone())
    }

    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        Err(PortError::Other("not used".into()))
    }

    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        Ok(Vec::new())
    }

    async fn estimate(&self, spec: &JobSpec) -> Result<Cost, PortError> {
        self.log.push("estimate");
        let preliminary = self.with(|state| state.preliminary.clone());
        Ok(preliminary
            .unwrap_or_else(|| Cost::usd(Decimal::new(130, 2) * Decimal::from(spec.variants.max(1)), false)))
    }

    async fn server_estimate(
        &self,
        _spec: &JobSpec,
        _inputs: &RemoteInputs,
    ) -> Result<Option<Cost>, PortError> {
        let (quote, error) = self.with(|state| (state.quote.clone(), state.quote_error.clone()));
        if quote.is_some() || error.is_some() {
            self.log.push("serverEstimate");
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(quote)
    }

    async fn charge(&self, _job: &Job) -> Result<Option<Cost>, PortError> {
        self.log.push("charge");
        if let Some(error) = self.with(|state| state.charge_error.clone()) {
            return Err(error);
        }
        Ok(self.with(|state| state.charge.clone()))
    }

    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        self.log.push("upload");
        let error = self.with(|state| state.upload_errors.pop_front());
        if let Some(error) = error {
            return Err(error);
        }
        self.with(|state| state.uploads.push(file.content_hash.clone()));
        Ok(RemoteAsset::new(
            Url::parse(&format!("https://upload.example/{}", file.content_hash)).unwrap(),
        ))
    }

    async fn linked_asset(
        &self,
        origin: &Url,
        kind: MediaLinkKind,
        _byte_count: i64,
    ) -> Result<Option<RemoteAsset>, PortError> {
        self.log.push(format!("linked {}", kind.raw_value()));
        if !self.with(|state| state.linked) {
            return Ok(None);
        }
        Ok(Some(RemoteAsset::new(origin.clone())))
    }

    async fn server_asset(&self, media: &jutsu_core::ServerMedia) -> Result<Option<RemoteAsset>, PortError> {
        self.log.push("server media");
        // Only media its own account listed, as the confirmed account takes them.
        if !self.with(|state| state.server_media && media.owner == state.owner) {
            return Ok(None);
        }
        let mut asset = RemoteAsset::new(media.url.clone());
        asset.id = Some(media.id.clone());
        Ok(Some(asset))
    }

    async fn submit(&self, spec: &JobSpec, inputs: &RemoteInputs, key: Uuid) -> Result<RemoteJob, PortError> {
        self.log.push("submit");
        let step = self.with(|state| {
            state.submit_keys.push(key);
            state.submitted_inputs.push(inputs.clone());
            state.submit_steps.pop_front()
        });
        match step {
            Some(SubmitStep::Fail(error)) => Err(error),
            Some(SubmitStep::Hang) => {
                self.accept(key, spec);
                std::future::pending().await
            }
            Some(SubmitStep::Answer(gate)) => {
                let remote = self.accept(key, spec);
                gate.wait().await;
                Ok(remote)
            }
            None => Ok(self.accept(key, spec)),
        }
    }

    async fn submit_is_idempotent(&self, _spec: &JobSpec) -> Result<bool, PortError> {
        Ok(!self.with(|state| state.not_idempotent))
    }

    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        self.log.push("status");
        let mut state = self.state.lock().unwrap();
        state.status_asked.push(job.request_id.clone());
        let status = match state.status_steps.pop_front() {
            Some(step) => step?,
            None => state
                .held_status
                .clone()
                .unwrap_or_else(|| RemoteStatus::Completed {
                    video_url: video_url(),
                }),
        };
        if !matches!(status, RemoteStatus::Queued | RemoteStatus::InProgress) {
            state.in_flight.remove(&job.request_id);
        }
        Ok(status)
    }

    fn status_batch_limit(&self) -> usize {
        self.with(|state| state.status_batch_limit)
    }

    async fn statuses(&self, jobs: &[RemoteJob]) -> Result<Option<jutsu_core::BatchStatus>, PortError> {
        self.log.push("statuses");
        let mut statuses = std::collections::BTreeMap::new();
        let ids: Vec<String> = jobs.iter().map(|job| job.request_id.clone()).collect();
        let held = self.with(|state| {
            state.batch_calls.push(ids);
            match &state.batch_mode {
                BatchMode::Answer => Ok(None),
                BatchMode::Unavailable => Err(None),
                BatchMode::Fail(error) => Err(Some(error.clone())),
                BatchMode::Held(gate) => Ok(Some(gate.clone())),
            }
        });
        match held {
            Ok(Some(gate)) => gate.wait().await,
            Ok(None) => {}
            Err(None) => return Ok(None),
            Err(Some(error)) => return Err(error),
        }
        let mut state = self.state.lock().unwrap();
        for job in jobs {
            if state.batch_omits.contains(&job.request_id) {
                continue;
            }
            let status = state
                .batch_statuses
                .get(&job.request_id)
                .or(state.held_status.as_ref())
                .cloned()
                .unwrap_or_else(|| RemoteStatus::Completed {
                    video_url: video_url(),
                });
            if !matches!(status, RemoteStatus::Queued | RemoteStatus::InProgress) {
                state.in_flight.remove(&job.request_id);
            }
            statuses.insert(job.request_id.clone(), status);
        }
        Ok(Some(jutsu_core::BatchStatus {
            statuses,
            poll_after: state.batch_poll_after,
        }))
    }

    async fn cancel(&self, job: &RemoteJob) -> Result<(), PortError> {
        self.log.push("cancel");
        self.with(|state| {
            state.cancels.push(job.request_id.clone());
            state.in_flight.remove(&job.request_id);
        });
        Ok(())
    }

    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        Ok(None)
    }

    async fn pays_unlimited(&self, _spec: &JobSpec) -> bool {
        self.with(|state| state.pays_unlimited)
    }
}

/// How the fake media adds sound to a result.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioMode {
    /// The result already has sound.
    Keep,
    /// Writes the result plus "+audio" to the output.
    Mux,
    Fail,
    /// The first call finds the disk full, later ones mux.
    DiskFullOnce,
}

#[derive(Clone)]
pub(crate) struct FakeMedia {
    log: CallLog,
    pub(crate) audio: AudioMode,
    pub(crate) progress_steps: Vec<f64>,
    pub(crate) image_size: Option<PixelSize>,
    pub(crate) output_media: Option<OutputMedia>,
}

#[async_trait]
impl MediaPreparing for FakeMedia {
    async fn prepare_video(
        &self,
        source: &SourceClip,
        progress: Progress<f64>,
    ) -> Result<PreparedFile, PortError> {
        self.log.push("prepareVideo");
        for step in &self.progress_steps {
            progress(*step);
        }
        Ok(prepared(&source.file_url, "video/mp4", "video"))
    }

    async fn prepare_image(&self, image: &LocalImage) -> Result<PreparedFile, PortError> {
        self.log.push("prepareImage");
        let name = image
            .file_url
            .path_segments()
            .and_then(Iterator::last)
            .unwrap_or("image")
            .to_owned();
        Ok(prepared(&image.file_url, "image/jpeg", &name))
    }

    async fn prepare_audio(&self, audio: &LocalAudio) -> Result<PreparedFile, PortError> {
        self.log.push("prepareAudio");
        Ok(prepared(&audio.file_url, "audio/mp4", "audio"))
    }

    async fn restore_audio(&self, _spec: &JobSpec, result: &Url, output: &Url) -> Result<bool, PortError> {
        self.log.push("restoreAudio");
        let mux = || {
            let mut data = std::fs::read(result.to_file_path().unwrap()).unwrap();
            data.extend_from_slice(b"+audio");
            std::fs::write(output.to_file_path().unwrap(), data).unwrap();
            Ok(true)
        };
        match self.audio {
            AudioMode::Keep => Ok(false),
            AudioMode::Mux => mux(),
            AudioMode::Fail => Err(PortError::Other("mux failed".into())),
            AudioMode::DiskFullOnce if self.log.count("restoreAudio") == 1 => {
                Err(PortError::Failure(FailureReason::NotEnoughDiskSpace))
            }
            AudioMode::DiskFullOnce => mux(),
        }
    }

    async fn image_size(&self, _image: &LocalImage) -> Option<PixelSize> {
        self.image_size
    }

    async fn output_media(&self, _file: &Url) -> Option<OutputMedia> {
        self.output_media
    }
}

fn prepared(url: &Url, content_type: &str, hash: &str) -> PreparedFile {
    PreparedFile {
        file_url: url.clone(),
        content_type: content_type.into(),
        byte_count: 1,
        content_hash: hash.into(),
    }
}

/// Uploads remembered by key, with when they were made.
#[derive(Clone, Default)]
pub(crate) struct FakeUploadCache(pub(crate) Arc<Mutex<HashMap<String, (RemoteAsset, OffsetDateTime)>>>);

#[async_trait]
impl UploadCache for FakeUploadCache {
    async fn asset(
        &self,
        content_hash: &str,
        uploaded_after: OffsetDateTime,
    ) -> Result<Option<RemoteAsset>, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(content_hash)
            .filter(|(_, at)| *at >= uploaded_after)
            .map(|(asset, _)| asset.clone()))
    }

    async fn store(
        &self,
        asset: &RemoteAsset,
        content_hash: &str,
        uploaded_at: OffsetDateTime,
    ) -> Result<(), PortError> {
        self.0
            .lock()
            .unwrap()
            .insert(content_hash.into(), (asset.clone(), uploaded_at));
        Ok(())
    }

    async fn remove_assets(&self, uploaded_before: OffsetDateTime) -> Result<(), PortError> {
        self.0.lock().unwrap().retain(|_, (_, at)| *at >= uploaded_before);
        Ok(())
    }
}

/// Online or not, as the test sets it.
#[derive(Clone)]
pub(crate) struct FakeConnectivity(pub(crate) watch::Sender<bool>);

impl FakeConnectivity {
    pub(crate) fn set(&self, online: bool) {
        self.0.send_replace(online);
    }
}

impl Connectivity for FakeConnectivity {
    fn updates(&self) -> BoxStream<'static, bool> {
        let receiver = self.0.subscribe();
        Box::pin(futures::stream::unfold(
            (receiver, true),
            |(mut receiver, first)| async move {
                if !first && receiver.changed().await.is_err() {
                    return None;
                }
                let online = *receiver.borrow_and_update();
                Some((online, (receiver, false)))
            },
        ))
    }
}

/// How the fake downloader answers.
#[derive(Clone)]
pub(crate) enum DownloadMode {
    Complete,
    /// The first call writes 3 bytes and loses the network.
    DropOnce,
    /// Every call writes 3 bytes and never finishes.
    Hang,
    HangOnce,
    /// The first calls answer with these statuses.
    Refuse(Vec<u16>),
    FailOnce(fn() -> DownloadError),
    /// The first call writes 3 bytes and finishes once the gate opens; later ones complete.
    HoldOnce(Gate),
}

#[derive(Clone)]
pub(crate) struct FakeDownloader {
    log: CallLog,
    pub(crate) mode: DownloadMode,
    /// The file's size at the start of every call.
    pub(crate) starts: CallLog,
}

const BODY: &[u8] = b"video";

#[async_trait]
impl Downloader for FakeDownloader {
    async fn download(&self, _url: &Url, file: &Path) -> Result<(), DownloadError> {
        self.log.push("download");
        let calls = self.log.count("download");
        let existing = std::fs::read(file).unwrap_or_default();
        self.starts.push(existing.len().to_string());
        let complete = || {
            std::fs::write(file, BODY).map_err(|error| DownloadError::Io {
                kind: error.kind(),
                message: error.to_string(),
            })
        };
        let partial = || std::fs::write(file, &BODY[..3]).unwrap();
        match &self.mode {
            DownloadMode::DropOnce if calls == 1 => {
                partial();
                Err(DownloadError::Provider(ProviderError::Network))
            }
            DownloadMode::Hang => {
                partial();
                std::future::pending().await
            }
            DownloadMode::HangOnce if calls == 1 => {
                partial();
                std::future::pending().await
            }
            DownloadMode::Refuse(statuses) if calls <= statuses.len() => Err(DownloadError::Failed {
                status: statuses[calls - 1],
            }),
            DownloadMode::FailOnce(error) if calls == 1 => {
                partial();
                Err(error())
            }
            DownloadMode::HoldOnce(gate) if calls == 1 => {
                partial();
                gate.wait().await;
                complete()
            }
            _ => complete(),
        }
    }
}

/// All fakes for one test; engines made from it share them, as two launches of the app would.
pub(crate) struct Harness {
    pub(crate) log: CallLog,
    pub(crate) clock: Arc<TestClock>,
    pub(crate) provider: FakeProvider,
    pub(crate) repository: InMemoryRepository,
    pub(crate) cache: FakeUploadCache,
    pub(crate) connectivity: FakeConnectivity,
    pub(crate) output: tempfile::TempDir,
    pub(crate) max_concurrency: usize,
    pub(crate) downloader: FakeDownloader,
    pub(crate) media: FakeMedia,
    pub(crate) spend_status: Option<SpendStatus>,
    pub(crate) generation_deadline: Duration,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let log = CallLog::default();
        Self {
            clock: Arc::new(TestClock::new()),
            provider: FakeProvider::new(log.clone()),
            repository: InMemoryRepository::new(log.clone()),
            cache: FakeUploadCache::default(),
            connectivity: FakeConnectivity(watch::Sender::new(true)),
            output: tempfile::tempdir().unwrap(),
            max_concurrency: 2,
            downloader: FakeDownloader {
                log: log.clone(),
                mode: DownloadMode::Complete,
                starts: CallLog::default(),
            },
            media: FakeMedia {
                log: log.clone(),
                audio: AudioMode::Keep,
                progress_steps: vec![0.5, 1.0],
                image_size: None,
                output_media: None,
            },
            spend_status: None,
            generation_deadline: Duration::MAX,
            log,
        }
    }

    pub(crate) fn output_directory(&self) -> PathBuf {
        self.output.path().join("Videos")
    }

    pub(crate) fn settings(&self) -> EngineSettings {
        let mut settings = EngineSettings::new(self.output_directory());
        settings.max_concurrency = self.max_concurrency;
        settings.jitter = 0.0;
        settings.upload_max_age = Duration::hours(1);
        settings.generation_deadline = self.generation_deadline;
        settings
    }

    pub(crate) fn deps(&self, agent_policy: Option<AgentSpendPolicy>) -> EngineDeps {
        let mut deps = EngineDeps::new(
            Arc::new(self.repository.clone()),
            Arc::new(self.cache.clone()),
            Arc::new(self.media.clone()),
            Arc::new(self.connectivity.clone()),
        );
        deps.provider = Some(Arc::new(self.provider.clone()));
        deps.downloader = Some(Arc::new(self.downloader.clone()));
        deps.clock = self.clock.clone();
        deps.random = Arc::new(|| 0.5);
        if let Some(status) = self.spend_status {
            deps.spend_status = Some(Arc::new(move |_| Box::pin(async move { Ok(Some(status)) })));
        }
        deps.agent_policy = agent_policy;
        deps
    }

    /// A started engine over the fakes.
    pub(crate) async fn engine(&self) -> JobEngine {
        self.engine_with(self.deps(None)).await
    }

    pub(crate) async fn engine_with(&self, deps: EngineDeps) -> JobEngine {
        let engine = JobEngine::new(deps, self.settings());
        engine.start().await.unwrap();
        engine
    }
}

/// A motion transfer from a local clip with `references` local images.
pub(crate) fn make_spec(references: usize) -> JobSpec {
    let source = SourceClip::new(
        Url::from_file_path(std::env::temp_dir().join("source.mov")).unwrap(),
        60.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 10.0),
    );
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        Mode::MotionTransfer.default_model(),
        Some(source),
        Resolution::P480,
    );
    spec.references = (0..references)
        .map(|index| {
            LocalImage::new(
                Url::from_file_path(std::env::temp_dir().join(format!("ref{index}.jpg"))).unwrap(),
            )
        })
        .collect();
    spec
}

/// Lets stray tasks run; only for asserting that nothing more happens.
pub(crate) async fn quiesce() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}

pub(crate) fn paused(reason: PauseReason) -> JobState {
    JobState::Paused(reason)
}

pub(crate) fn failed(reason: FailureReason) -> JobState {
    JobState::Failed { reason }
}

/// Today's spend and a cap in dollars.
pub(crate) fn spend(committed_today: &str, cap: &str) -> SpendStatus {
    SpendStatus {
        committed_today: committed_today.parse().unwrap(),
        cap: Some(jutsu_core::SpendCap::usd(cap.parse().unwrap())),
    }
}

/// A spend check that answers what `read` returns, as Swift's `readSpend`.
pub(crate) fn read_spend(
    read: impl Fn() -> Result<Option<SpendStatus>, jutsu_core::BudgetError> + Send + Sync + 'static,
) -> crate::engine::SpendStatusFn {
    let read = Arc::new(read);
    Arc::new(move |_| {
        let answer = read();
        Box::pin(async move { answer })
    })
}

/// An exact price in credits.
pub(crate) fn credits(amount: i64) -> Cost {
    Cost::credits(Decimal::from(amount), false)
}

/// Waits until `condition` holds, letting the engine's tasks run; fails the test after 10 s.
pub(crate) async fn eventually(condition: impl Fn() -> bool) {
    let waited = tokio::time::timeout(StdDuration::from_secs(10), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the condition never held");
}
