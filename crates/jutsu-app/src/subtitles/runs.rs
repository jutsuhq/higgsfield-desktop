//! Subtitles made on this computer (`SubtitleRuns.swift`): one run at a time, each hearing the file of the exact
//! version it was started on and saving its subtitles on that version. Free, so neither the queue nor the daily cap
//! sees them. A cancelled or failed run saves nothing.
//!
//! The runs have a queue of their own, as on macOS: Enhance's runs on this computer (`OnDeviceRuns`) are another one.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use futures::future::BoxFuture;
use jutsu_core::{
    MaterialReference, OnDeviceProgress, PortError, Progress, SpeechAvailability, SpeechModel,
    SpeechSelection, SpeechTranscribing, SubtitleEngine, Subtitles, TranscriptionError,
};
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::text;
use crate::connection::UserMessage;
use crate::preferences::PreferencesStore;

/// What a run hears: the exact version the subtitles are for, its file, the language and the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubtitleRequest {
    pub source: MaterialReference,
    pub file: PathBuf,
    /// BCP 47, "en-US".
    pub language: String,
    pub engine: SubtitleEngine,
}

impl SubtitleRequest {
    pub fn new(source: MaterialReference, file: impl Into<PathBuf>, language: &str) -> Self {
        Self {
            source,
            file: file.into(),
            language: language.to_owned(),
            engine: SubtitleEngine::BuiltIn,
        }
    }
}

/// How far a run is (`OnDeviceRuns.State`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SubtitleRunState {
    Waiting,
    /// 0…1 of the model's download.
    DownloadingModel {
        fraction: f64,
    },
    /// 0…1 of the words.
    Working {
        fraction: f64,
    },
}

impl SubtitleRunState {
    /// The share done, `None` while it waits.
    pub fn fraction(self) -> Option<f64> {
        match self {
            Self::Waiting => None,
            Self::DownloadingModel { fraction } | Self::Working { fraction } => Some(fraction),
        }
    }
}

/// One run, waiting or running.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleRun {
    pub id: Uuid,
    pub request: SubtitleRequest,
    /// The model that hears it.
    pub model: SpeechModel,
    pub state: SubtitleRunState,
    pub created_at: OffsetDateTime,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum SubtitleOutcome {
    Done(Box<Subtitles>),
    Failed(String),
    Canceled,
}

/// A run that ended, as `jobs` reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedRun {
    pub run: SubtitleRun,
    pub outcome: SubtitleOutcome,
    pub finished_at: OffsetDateTime,
}

/// Saves the subtitles on `request.source`; an error's description fails the run.
pub type Register =
    Arc<dyn Fn(SubtitleRequest, Subtitles) -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

#[derive(Default)]
struct Inner {
    /// Waiting and running, in the order they run: the first one is running.
    runs: Vec<SubtitleRun>,
    failure: Option<UserMessage>,
    availability: Option<SpeechAvailability>,
    /// Languages whose model a run found missing although the engine listed it as installed.
    missing_models: BTreeSet<String>,
    finished: Vec<FinishedRun>,
    /// The run being heard and how to stop it.
    running: Option<(Uuid, CancellationToken)>,
    /// The loops still working, the availability read after a run included.
    loops: usize,
    /// The system's preferred languages, first first.
    system_languages: Vec<String>,
}

/// The runs, one at a time.
pub struct SubtitleRuns {
    me: Weak<SubtitleRuns>,
    engine: Arc<dyn SpeechTranscribing>,
    preferences: Arc<PreferencesStore>,
    register: Register,
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for SubtitleRuns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SubtitleRuns").finish_non_exhaustive()
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

impl SubtitleRuns {
    /// How many ended runs are kept for `jobs`.
    pub const KEPT_FINISHED: usize = 100;
    pub const LAST_LANGUAGE_KEY: &'static str = "subtitles.lastLanguage";
    pub const ENGINE_KEY: &'static str = "subtitles.engine";

    /// `preferences` keeps the person's agreement to model downloads, the last language and the default engine.
    pub fn new(
        engine: Arc<dyn SpeechTranscribing>,
        preferences: Arc<PreferencesStore>,
        register: Register,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            engine,
            preferences,
            register,
            inner: Mutex::new(Inner {
                system_languages: system_languages(),
                ..Inner::default()
            }),
            changes: watch::channel(0).0,
        })
    }

    fn locked(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    /// A revision that goes up on every change of the runs, their states or the availability.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    // MARK: Settings

    /// The model the Subtitles sheet and agents start on, from Settings › Subtitles; a run may pick another.
    pub fn default_engine(&self) -> SubtitleEngine {
        self.preferences
            .value(Self::ENGINE_KEY)
            .as_ref()
            .and_then(Value::as_str)
            .and_then(SubtitleEngine::from_raw)
            .unwrap_or(SubtitleEngine::Auto)
    }

    pub fn set_default_engine(&self, engine: SubtitleEngine) {
        self.write(Self::ENGINE_KEY, Value::from(engine.raw_value()));
        self.changed();
    }

    fn write(&self, key: &str, value: Value) {
        if let Err(error) = self.preferences.set_value(key, Some(value)) {
            tracing::error!(%error, key, "can't save a subtitles setting");
        }
    }

    /// The system's preferred languages, as the webview lists them (`navigator.languages`).
    pub fn set_system_languages(&self, languages: Vec<String>) {
        if !languages.is_empty() {
            self.locked().system_languages = languages;
        }
    }

    /// The language the sheet starts on: the last one a run was started in, then the system's.
    pub fn preferred_language(&self) -> Option<String> {
        let last = self
            .preferences
            .value(Self::LAST_LANGUAGE_KEY)
            .and_then(|value| value.as_str().map(str::to_owned));
        let system = self.locked().system_languages.clone();
        SpeechSelection::preferred(&self.languages(), last.as_deref(), &system)
    }

    // MARK: Availability

    /// Reads which models and languages this computer has, which downloads nothing.
    pub async fn load_availability(&self) {
        let availability = self.engine.availability().await;
        self.locked().availability = Some(availability);
        self.changed();
    }

    pub fn availability(&self) -> Option<SpeechAvailability> {
        self.locked().availability.clone()
    }

    /// The languages the Subtitles sheet offers; none until `load_availability` read them.
    pub fn languages(&self) -> Vec<String> {
        self.locked()
            .availability
            .as_ref()
            .map(SpeechSelection::languages)
            .unwrap_or_default()
    }

    /// The model `engine` would hear `language` with; `None` when it doesn't take it, or before `load_availability`.
    pub fn model(&self, language: &str, engine: SubtitleEngine) -> Option<SpeechModel> {
        let inner = self.locked();
        let availability = inner.availability.as_ref()?;
        SpeechSelection::model(engine, language, availability)
    }

    /// Whether a run in `language` waits for the person to agree to downloading that language's model: only for a
    /// model a run downloads that isn't here, or that a run found missing, before they agreed for the language.
    pub fn needs_consent(&self, language: &str, engine: SubtitleEngine) -> bool {
        if self.agreed_to_model(language) {
            return false;
        }
        let Some(model) = self.model(language, engine) else {
            return true;
        };
        if !model.asks_consent() {
            return false;
        }
        let inner = self.locked();
        let Some(availability) = inner.availability.as_ref() else {
            return true;
        };
        !availability.is_installed(model, language) || inner.missing_models.contains(language)
    }

    /// Whether `engine` hears `language` with a model Settings downloads, and it isn't downloaded yet.
    pub fn needs_download(&self, language: &str, engine: SubtitleEngine) -> bool {
        let Some(model) = self.model(language, engine).filter(|model| !model.asks_consent()) else {
            return false;
        };
        let inner = self.locked();
        inner
            .availability
            .as_ref()
            .is_some_and(|availability| !availability.is_installed(model, language))
    }

    fn consent_key(language: &str) -> String {
        format!("onDevice.modelConsent.speech.{language}")
    }

    /// Whether the person agreed to downloading the speech model of `language`. They agree per language, as macOS
    /// downloads a model per locale.
    pub fn agreed_to_model(&self, language: &str) -> bool {
        self.preferences
            .value(&Self::consent_key(language))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    pub fn agree_to_model(&self, language: &str) {
        self.write(&Self::consent_key(language), Value::Bool(true));
        self.changed();
    }

    // MARK: Runs

    /// Waiting and running, in the order they run.
    pub fn runs(&self) -> Vec<SubtitleRun> {
        self.locked().runs.clone()
    }

    /// What went wrong with the last run that failed, until it is taken to be shown.
    pub fn take_failure(&self) -> Option<UserMessage> {
        self.locked().failure.take()
    }

    pub fn failure(&self) -> Option<UserMessage> {
        self.locked().failure.clone()
    }

    /// The first run, waiting or running, started on `material_id`.
    pub fn run_for(&self, material_id: Uuid) -> Option<SubtitleRun> {
        self.locked()
            .runs
            .iter()
            .find(|run| run.request.source.material_id == material_id)
            .cloned()
    }

    /// The run waiting or running as `id`, else the one that ended so, while it is kept.
    pub fn run(&self, id: Uuid) -> Option<(SubtitleRun, Option<FinishedRun>)> {
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

    /// The runs that ended, oldest first.
    pub fn finished(&self) -> Vec<FinishedRun> {
        self.locked().finished.clone()
    }

    /// Refused, returning `false`, when the engine doesn't take the language, while its model download waits for the
    /// person's agreement or Parakeet isn't downloaded, and while a run on the same item waits or runs.
    /// `remembers_language`: whether the sheet starts on this run's language next time: the person's runs only.
    pub fn start(&self, request: SubtitleRequest, id: Uuid, remembers_language: bool) -> bool {
        let Some(model) = self.model(&request.language, request.engine) else {
            return false;
        };
        if self.needs_consent(&request.language, request.engine)
            || self.needs_download(&request.language, request.engine)
            || self.run_for(request.source.material_id).is_some()
        {
            return false;
        }
        if remembers_language {
            self.write(Self::LAST_LANGUAGE_KEY, Value::from(request.language.clone()));
        }
        self.locked().runs.push(SubtitleRun {
            id,
            request,
            model,
            state: SubtitleRunState::Waiting,
            created_at: OffsetDateTime::now_utc(),
        });
        self.next();
        self.changed();
        true
    }

    /// A waiting run is dropped; the running one stops.
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
                    end(&mut inner, run, SubtitleOutcome::Canceled);
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

    /// Cancels the runs on `material_ids` and returns once they are over, so none saves afterwards.
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
            me.load_availability().await;
            me.locked().loops -= 1;
            me.changed();
        });
    }

    fn update(&self, id: Uuid, state: SubtitleRunState) {
        {
            let mut inner = self.locked();
            let Some(run) = inner.runs.iter_mut().find(|run| run.id == id) else {
                return;
            };
            run.state = state;
        }
        self.changed();
    }

    async fn execute(&self, run: &SubtitleRun, cancel: &CancellationToken) -> SubtitleOutcome {
        let request = run.request.clone();
        let may_download = self.agreed_to_model(&request.language);
        let (model_throttle, words_throttle) = (Throttle::default(), Throttle::default());
        let me = self.me.clone();
        let id = run.id;
        let progress: Progress<OnDeviceProgress> = Arc::new(move |progress| {
            let state = match progress {
                OnDeviceProgress::DownloadingModel(fraction) => {
                    if !model_throttle.admits(fraction) {
                        return;
                    }
                    SubtitleRunState::DownloadingModel { fraction }
                }
                OnDeviceProgress::Working(fraction) => {
                    if !words_throttle.admits(fraction) {
                        return;
                    }
                    SubtitleRunState::Working { fraction }
                }
            };
            if let Some(runs) = me.upgrade() {
                runs.update(id, state);
            }
        });
        self.update(id, SubtitleRunState::Working { fraction: 0.0 });
        let Ok(file) = Url::from_file_path(&request.file) else {
            return self.failed(
                &request,
                &PortError::Transcription(TranscriptionError::CantOpenFile),
            );
        };
        let heard = self
            .engine
            .transcribe(&file, &request.language, run.model, may_download, progress);
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => return SubtitleOutcome::Canceled,
            result = heard => result,
        };
        if cancel.is_cancelled() {
            return SubtitleOutcome::Canceled;
        }
        let transcript = match result {
            Ok(transcript) => transcript,
            Err(PortError::Cancelled) => return SubtitleOutcome::Canceled,
            Err(error) => return self.failed(&request, &error),
        };
        let subtitles = Subtitles::new(transcript, None, OffsetDateTime::now_utc());
        match (self.register)(request.clone(), subtitles.clone()).await {
            Ok(()) => SubtitleOutcome::Done(Box::new(subtitles)),
            Err(error) => self.failed(&request, &PortError::Other(error)),
        }
    }

    fn failed(&self, request: &SubtitleRequest, error: &PortError) -> SubtitleOutcome {
        let message = text::message(error);
        let mut inner = self.locked();
        if *error == PortError::Transcription(TranscriptionError::ModelNeeded) {
            inner.missing_models.insert(request.language.clone());
        }
        if !matches!(error, PortError::Transcription(_)) {
            tracing::error!(%error, "a subtitles run failed");
        }
        inner.failure = Some(UserMessage::new(text::FAILURE_TITLE, message.clone(), None));
        SubtitleOutcome::Failed(message)
    }
}

fn end(inner: &mut Inner, run: SubtitleRun, outcome: SubtitleOutcome) {
    inner.finished.push(FinishedRun {
        run,
        outcome,
        finished_at: OffsetDateTime::now_utc(),
    });
    let excess = inner.finished.len().saturating_sub(SubtitleRuns::KEPT_FINISHED);
    inner.finished.drain(..excess);
}

/// The system's preferred languages from the locale variables (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`), as
/// BCP 47: "de_AT.UTF-8" is "de-AT". The webview's own list replaces them once the window sends it.
fn system_languages() -> Vec<String> {
    let mut languages = Vec::new();
    for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        for part in value.split(':') {
            let tag = part.split(['.', '@']).next().unwrap_or("").replace('_', "-");
            if !tag.is_empty() && tag != "C" && tag != "POSIX" && !languages.contains(&tag) {
                languages.push(tag);
            }
        }
    }
    languages
}
