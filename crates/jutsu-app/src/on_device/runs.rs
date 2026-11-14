//! Enhance's runs on this computer (`OnDeviceRuns.swift`): one at a time, each adding a new version to the item it was
//! started on, linked to the exact version it was made from; that version and its file stay as they are. Free, so
//! neither the queue nor the daily cap sees them. A cancelled or failed run leaves no file behind.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use futures::future::BoxFuture;
use jutsu_core::{
    MaterialReference, OnDeviceAction, OnDeviceEngine, OnDeviceEnhancing, OnDeviceOutput, OnDeviceProgress,
    PortError, Progress, unused_file_path,
};
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::text;
use crate::connection::UserMessage;
use crate::preferences::PreferencesStore;

/// What a run makes, from which version and file, and where its result goes.
#[derive(Debug, Clone, PartialEq)]
pub struct OnDeviceRequest {
    /// The exact version the result is made from.
    pub source: MaterialReference,
    pub file: PathBuf,
    pub action: OnDeviceAction,
    /// The folder the result goes in.
    pub destination: PathBuf,
    /// Where the result is written until it is done, as a hidden `.ondevice-<UUID>.partial` (`partial_name`): on the
    /// destination's volume, so it moves in at once. `remove_leftovers` clears the ones a crash left, at launch.
    pub scratch: PathBuf,
}

/// How far a run is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnDeviceRunState {
    Waiting,
    /// 0…1 of the engine's download, before its first run.
    DownloadingModel(f64),
    /// 0…1.
    Working(f64),
}

impl OnDeviceRunState {
    pub fn is_downloading_model(self) -> bool {
        matches!(self, Self::DownloadingModel(_))
    }

    /// How far the download or the frames are; `None` while waiting.
    pub fn fraction(self) -> Option<f64> {
        match self {
            Self::Waiting => None,
            Self::DownloadingModel(fraction) | Self::Working(fraction) => Some(fraction),
        }
    }
}

/// One run, waiting or running.
#[derive(Debug, Clone, PartialEq)]
pub struct OnDeviceRun {
    pub id: Uuid,
    pub request: OnDeviceRequest,
    pub state: OnDeviceRunState,
    pub created_at: OffsetDateTime,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum OnDeviceOutcome {
    /// The result's file, now the item's current version.
    Done(PathBuf),
    Failed(String),
    Canceled,
}

/// A run that ended, as `jobs` reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedOnDeviceRun {
    pub run: OnDeviceRun,
    pub outcome: OnDeviceOutcome,
    pub finished_at: OffsetDateTime,
}

/// Adds the finished file as the new version of `request.source`'s material; an error's description fails the run.
pub type Register = Arc<
    dyn Fn(OnDeviceRequest, PathBuf, OnDeviceOutput) -> BoxFuture<'static, Result<(), String>> + Send + Sync,
>;

#[derive(Default)]
struct Inner {
    runs: Vec<OnDeviceRun>,
    failure: Option<UserMessage>,
    finished: Vec<FinishedOnDeviceRun>,
    /// Whether each engine's model was here when a run on it last ended: never asked outside a run.
    ready_models: HashMap<OnDeviceEngine, bool>,
    running: Option<(Uuid, CancellationToken)>,
    loops: usize,
}

/// The person's agreement to downloading an engine's model (`OnDeviceModelConsent`), asked once per engine before its
/// first run and remembered in the settings under the macOS app's key.
#[derive(Clone)]
pub struct OnDeviceModelConsent {
    preferences: Arc<PreferencesStore>,
}

impl OnDeviceModelConsent {
    pub fn new(preferences: Arc<PreferencesStore>) -> Self {
        Self { preferences }
    }

    /// "onDevice.modelConsent.<engine>".
    pub fn key(engine: OnDeviceEngine) -> String {
        format!("onDevice.modelConsent.{}", engine.raw_value())
    }

    /// Whether a run on `engine` may start: one without a model always may.
    pub fn allows(&self, engine: OnDeviceEngine) -> bool {
        !engine.downloads_model()
            || self
                .preferences
                .value(&Self::key(engine))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
    }

    pub fn agree(&self, engine: OnDeviceEngine) {
        if let Err(error) = self
            .preferences
            .set_value(&Self::key(engine), Some(Value::Bool(true)))
        {
            tracing::error!(%error, "can't save the agreement to a model download");
        }
    }
}

/// Lets a progress through only when its whole percent changed (`ProgressThrottle`).
#[derive(Default)]
struct Throttle(Mutex<Option<i64>>);

impl Throttle {
    fn admits(&self, fraction: f64) -> bool {
        #[allow(clippy::cast_possible_truncation, reason = "a percent")]
        let percent = (fraction * 100.0).floor() as i64;
        let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *last == Some(percent) {
            return false;
        }
        *last = Some(percent);
        true
    }
}

/// The runs, one at a time.
pub struct OnDeviceRuns {
    me: Weak<OnDeviceRuns>,
    engine: Arc<dyn OnDeviceEnhancing>,
    consent: OnDeviceModelConsent,
    register: Register,
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for OnDeviceRuns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OnDeviceRuns").finish_non_exhaustive()
    }
}

impl OnDeviceRuns {
    /// How many ended runs are kept for `jobs`.
    pub const KEPT_FINISHED: usize = 100;
    pub const PARTIAL_PREFIX: &'static str = ".ondevice-";

    pub fn new(
        engine: Arc<dyn OnDeviceEnhancing>,
        consent: OnDeviceModelConsent,
        register: Register,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            engine,
            consent,
            register,
            inner: Mutex::new(Inner::default()),
            changes: watch::channel(0).0,
        })
    }

    fn locked(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    /// A revision that goes up on every change of the runs or their states.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Whether this computer has the engine; asks nothing about its model.
    pub fn supports(&self, engine: OnDeviceEngine) -> bool {
        self.engine.supports(engine)
    }

    /// Whether a run on `engine` waits for the person to agree to downloading its model.
    pub fn needs_consent(&self, engine: OnDeviceEngine) -> bool {
        !self.consent.allows(engine)
    }

    pub fn agree(&self, engine: OnDeviceEngine) {
        self.consent.agree(engine);
        self.changed();
    }

    /// Whether the engine's model is known to be here: as a run on it found it, after the person agreed to its
    /// download; `false` before any such run since launch.
    pub fn model_is_ready(&self, engine: OnDeviceEngine) -> bool {
        self.consent.allows(engine) && self.locked().ready_models.get(&engine) == Some(&true)
    }

    /// Waiting and running, in the order they run: the first one is running.
    pub fn runs(&self) -> Vec<OnDeviceRun> {
        self.locked().runs.clone()
    }

    /// What went wrong with the last run that failed, until it is taken to be shown.
    pub fn take_failure(&self) -> Option<UserMessage> {
        let failure = self.locked().failure.take();
        if failure.is_some() {
            self.changed();
        }
        failure
    }

    pub fn failure(&self) -> Option<UserMessage> {
        self.locked().failure.clone()
    }

    /// The first run, waiting or running, started on `material_id`.
    pub fn run_for(&self, material_id: Uuid) -> Option<OnDeviceRun> {
        self.locked()
            .runs
            .iter()
            .find(|run| run.request.source.material_id == material_id)
            .cloned()
    }

    /// The run waiting or running as `id`, else the one that ended so, while it is kept.
    pub fn run(&self, id: Uuid) -> Option<(OnDeviceRun, Option<FinishedOnDeviceRun>)> {
        let inner = self.locked();
        if let Some(run) = inner.runs.iter().find(|run| run.id == id) {
            return Some((run.clone(), None));
        }
        inner
            .finished
            .iter()
            .rev()
            .find(|finished| finished.run.id == id)
            .map(|finished| (finished.run.clone(), Some(finished.clone())))
    }

    /// Refused, returning `false`, while the engine's model download waits for the person's agreement, and while a
    /// run on the same item waits or runs: each is made from the version it was started on, so runs aren't chained.
    /// `id` is the run's, which an agent's request records before it starts.
    pub fn start(&self, request: OnDeviceRequest, id: Uuid) -> bool {
        if !self.consent.allows(request.action.engine()) || self.run_for(request.source.material_id).is_some()
        {
            return false;
        }
        self.locked().runs.push(OnDeviceRun {
            id,
            request,
            state: OnDeviceRunState::Waiting,
            created_at: OffsetDateTime::now_utc(),
        });
        self.next();
        self.changed();
        true
    }

    /// A waiting run is dropped; the running one stops, and its file is deleted.
    pub fn cancel(&self, id: Uuid) {
        {
            let mut inner = self.locked();
            let Some(index) = inner.runs.iter().position(|run| run.id == id) else {
                return;
            };
            match &inner.running {
                Some((running, token)) if *running == id => token.cancel(),
                _ => {
                    let run = inner.runs.remove(index);
                    end(&mut inner, run, OnDeviceOutcome::Canceled);
                }
            }
        }
        self.changed();
    }

    /// Returns once the run `id` is over; at once when it isn't the running one.
    pub async fn over(&self, id: Uuid) {
        let mut changes = self.subscribe();
        loop {
            let running = self
                .locked()
                .running
                .as_ref()
                .is_some_and(|(running, _)| *running == id);
            if !running || changes.changed().await.is_err() {
                return;
            }
        }
    }

    /// Cancels the runs waiting or running on `material_ids` and returns once they are over, so none adds a version
    /// afterwards: Delete from Library calls it before it trashes the items.
    pub async fn stop(&self, material_ids: &[Uuid]) {
        let ids: Vec<Uuid> = self
            .runs()
            .iter()
            .filter(|run| material_ids.contains(&run.request.source.material_id))
            .map(|run| run.id)
            .collect();
        for id in &ids {
            self.cancel(*id);
        }
        for id in ids {
            self.over(id).await;
        }
    }

    /// Waits until every run started so far is over.
    pub async fn finish(&self) {
        let mut changes = self.subscribe();
        loop {
            if self.locked().loops == 0 || changes.changed().await.is_err() {
                return;
            }
        }
    }

    fn next(&self) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let (run, token) = {
            let mut inner = self.locked();
            if inner.running.is_some() {
                return;
            }
            let Some(run) = inner.runs.first().cloned() else {
                return;
            };
            let token = CancellationToken::new();
            inner.running = Some((run.id, token.clone()));
            inner.loops += 1;
            (run, token)
        };
        tokio::spawn(async move {
            let outcome = me.execute(&run, &token).await;
            {
                let mut inner = me.locked();
                if let Some(index) = inner.runs.iter().position(|ended| ended.id == run.id) {
                    let ended = inner.runs.remove(index);
                    end(&mut inner, ended, outcome);
                }
                inner.running = None;
            }
            me.changed();
            me.next();
            me.read_model(run.request.action.engine()).await;
            me.locked().loops -= 1;
            me.changed();
        });
    }

    /// Notes whether `engine`'s model is here, asked off the caller's task once a run on it ended: the run had the
    /// model or downloaded it with the person's agreement, so asking starts nothing.
    async fn read_model(&self, engine: OnDeviceEngine) {
        if !engine.downloads_model() || !self.consent.allows(engine) {
            return;
        }
        let enhancer = Arc::clone(&self.engine);
        let ready = tokio::task::spawn_blocking(move || enhancer.model_is_ready(engine))
            .await
            .unwrap_or(false);
        self.locked().ready_models.insert(engine, ready);
    }

    fn update(&self, id: Uuid, state: OnDeviceRunState) {
        {
            let mut inner = self.locked();
            let Some(run) = inner.runs.iter_mut().find(|run| run.id == id) else {
                return;
            };
            run.state = state;
        }
        self.changed();
    }

    async fn execute(&self, run: &OnDeviceRun, cancel: &CancellationToken) -> OnDeviceOutcome {
        let request = run.request.clone();
        let partial = request.scratch.join(Self::partial_name(run.id));
        let (model_throttle, frames_throttle) = (Throttle::default(), Throttle::default());
        let me = self.me.clone();
        let id = run.id;
        let progress: Progress<OnDeviceProgress> = Arc::new(move |progress| {
            let state = match progress {
                OnDeviceProgress::DownloadingModel(fraction) => {
                    if !model_throttle.admits(fraction) {
                        return;
                    }
                    OnDeviceRunState::DownloadingModel(fraction)
                }
                OnDeviceProgress::Working(fraction) => {
                    if !frames_throttle.admits(fraction) {
                        return;
                    }
                    OnDeviceRunState::Working(fraction)
                }
            };
            if let Some(runs) = me.upgrade() {
                runs.update(id, state);
            }
        });
        self.update(id, OnDeviceRunState::Working(0.0));
        let _ = std::fs::create_dir_all(&request.scratch);
        let (Ok(source), Ok(output)) = (Url::from_file_path(&request.file), Url::from_file_path(&partial))
        else {
            return self.failed(&request, "The item's file can't be read.");
        };
        let work = self.engine.run(&request.action, &source, &output, progress);
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(PortError::Cancelled),
            result = work => result,
        };
        let result = match result {
            Ok(output) if !cancel.is_cancelled() => Ok(output),
            Ok(_) => Err(PortError::Cancelled),
            Err(error) => Err(error),
        };
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                let _ = std::fs::remove_file(&partial);
                if error == PortError::Cancelled {
                    return OnDeviceOutcome::Canceled;
                }
                return self.failed(&request, &error.to_string());
            }
        };
        let file = match Self::place(&partial, &request, &output) {
            Ok(file) => file,
            Err(error) => {
                let _ = std::fs::remove_file(&partial);
                return self.failed(&request, &error.to_string());
            }
        };
        match (self.register)(request.clone(), file.clone(), output).await {
            Ok(()) => OnDeviceOutcome::Done(file),
            Err(error) => {
                // Not in the catalog, so taken back: a second try doesn't leave a second file.
                let _ = std::fs::remove_file(&file);
                self.failed(&request, &error)
            }
        }
    }

    fn failed(&self, request: &OnDeviceRequest, message: &str) -> OnDeviceOutcome {
        tracing::error!(error = message, "an on-device run failed");
        self.locked().failure = Some(UserMessage::new(
            &text::failure_title(&request.action),
            message.to_owned(),
            None,
        ));
        OnDeviceOutcome::Failed(message.to_owned())
    }

    /// ".ondevice-<UUID>.partial": not a job's scratch name (`.<UUID>.partial`), so the engine's sweep leaves it alone.
    pub fn partial_name(id: Uuid) -> String {
        format!(
            "{}{}.partial",
            Self::PARTIAL_PREFIX,
            id.as_hyphenated().to_string().to_uppercase()
        )
    }

    /// Deletes the partial files a crash or a quit left in `folder`. Call before the first run starts: it would
    /// delete a running one's too.
    pub fn remove_leftovers(folder: &Path) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(Self::PARTIAL_PREFIX) && name.ends_with(".partial") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// Moves the finished file in under a name of its own: "rooftop 60fps.mp4", "rooftop 60fps 2.mp4".
    pub fn place(
        partial: &Path,
        request: &OnDeviceRequest,
        output: &OnDeviceOutput,
    ) -> std::io::Result<PathBuf> {
        let stem = request
            .file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!(
            "{stem} {}.{}",
            request.action.file_suffix(),
            output.file_extension
        );
        std::fs::create_dir_all(&request.destination)?;
        let file = unused_file_path(&request.destination.join(name), Path::exists);
        std::fs::rename(partial, &file)?;
        Ok(file)
    }
}

fn end(inner: &mut Inner, run: OnDeviceRun, outcome: OnDeviceOutcome) {
    inner.finished.push(FinishedOnDeviceRun {
        run,
        outcome,
        finished_at: OffsetDateTime::now_utc(),
    });
    let excess = inner.finished.len().saturating_sub(OnDeviceRuns::KEPT_FINISHED);
    inner.finished.drain(..excess);
}
