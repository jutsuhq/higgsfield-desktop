//! The ported Swift tests of subtitles made on this computer: `SubtitleRunsTests`, `AgentSubtitleRunsTests` and
//! `SpeechModelsTests`. `theFileIsOpenedThroughItsBookmark` isn't ported: security-scoped bookmarks are macOS-only.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentCreated, AgentCue, AgentError, AgentSubtitleEngine,
    AgentSubtitleFormat, AgentSubtitlesAction, ErrorCode, SubtitlesCommand,
};
use jutsu_core::{
    MaterialReference, ModelManifest, OnDeviceProgress, PortError, Progress, SpeechAvailability, SpeechModel,
    SpeechModelDownloading, SpeechTranscribing, SubtitleCue, SubtitleEngine, Subtitles, TimedWord,
    Transcript, TranscriptionError, WhisperModel,
};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::agent::{AgentSubtitleRuns, Gate, ON_DEVICE_KIND, SubtitleTarget};
use super::models::{SpeechModelState, SpeechModels};
use super::runs::{Register, SubtitleOutcome, SubtitleRequest, SubtitleRunState, SubtitleRuns};
use super::service::{SubtitleSaveOutcome, SubtitleService};
use super::text;
use crate::agents::request_gate::Refusals;
use crate::agents::wire_id;
use crate::preferences::PreferencesStore;

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the fake heard.
#[derive(Debug, Clone, PartialEq)]
struct Heard {
    language: String,
    model: SpeechModel,
    may_download: bool,
}

/// Hears canned words and waits until released; notes which model heard what.
struct FakeSpeech {
    released: AtomicBool,
    running: AtomicUsize,
    most_at_once: AtomicUsize,
    heard: Mutex<Vec<Heard>>,
    available: Mutex<SpeechAvailability>,
    failure: Mutex<Option<PortError>>,
}

impl FakeSpeech {
    fn new() -> Arc<Self> {
        let mut available = SpeechAvailability::default();
        available
            .languages
            .insert(SpeechModel::Transcriber, vec!["en-US".to_owned()]);
        available.languages.insert(
            SpeechModel::Dictation,
            vec!["en-US".to_owned(), "ru-RU".to_owned()],
        );
        available
            .installed
            .insert(SpeechModel::Transcriber, BTreeSet::from(["en-US".to_owned()]));
        Arc::new(Self {
            released: AtomicBool::new(false),
            running: AtomicUsize::new(0),
            most_at_once: AtomicUsize::new(0),
            heard: Mutex::new(Vec::new()),
            available: Mutex::new(available),
            failure: Mutex::new(None),
        })
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    fn heard(&self) -> Vec<Heard> {
        locked(&self.heard).clone()
    }

    fn set_failure(&self, failure: Option<PortError>) {
        *locked(&self.failure) = failure;
    }

    fn edit(&self, edit: impl FnOnce(&mut SpeechAvailability)) {
        edit(&mut locked(&self.available));
    }
}

struct Running<'a>(&'a AtomicUsize);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl SpeechTranscribing for FakeSpeech {
    async fn availability(&self) -> SpeechAvailability {
        locked(&self.available).clone()
    }

    async fn transcribe(
        &self,
        _file: &Url,
        language: &str,
        model: SpeechModel,
        may_download: bool,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<Transcript, PortError> {
        let running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        let _running = Running(&self.running);
        self.most_at_once.fetch_max(running, Ordering::SeqCst);
        locked(&self.heard).push(Heard {
            language: language.to_owned(),
            model,
            may_download,
        });
        progress(OnDeviceProgress::Working(0.5));
        while !self.released.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        if let Some(failure) = locked(&self.failure).clone() {
            return Err(failure);
        }
        Ok(Transcript {
            language: language.to_owned(),
            model,
            revision: "1".to_owned(),
            words: vec![
                TimedWord::new("Hello", 0.2, 0.6),
                TimedWord::new("there.", 0.7, 1.1),
            ],
        })
    }
}

type Saved = Arc<Mutex<Vec<(MaterialReference, Subtitles)>>>;

struct Fixture {
    speech: Arc<FakeSpeech>,
    preferences: Arc<PreferencesStore>,
    runs: Arc<SubtitleRuns>,
    saved: Saved,
    _folder: tempfile::TempDir,
}

fn fixture() -> Fixture {
    fixture_with(FakeSpeech::new())
}

fn fixture_with(speech: Arc<FakeSpeech>) -> Fixture {
    let folder = tempfile::tempdir().expect("a temp dir");
    let preferences = Arc::new(PreferencesStore::open(folder.path()));
    let saved: Saved = Arc::default();
    let into = Arc::clone(&saved);
    let register: Register = Arc::new(move |request: SubtitleRequest, subtitles: Subtitles| {
        locked(&into).push((request.source, subtitles));
        Box::pin(async { Ok(()) })
    });
    let runs = SubtitleRuns::new(
        Arc::clone(&speech) as Arc<dyn SpeechTranscribing>,
        Arc::clone(&preferences),
        register,
    );
    Fixture {
        speech,
        preferences,
        runs,
        saved,
        _folder: folder,
    }
}

fn source() -> MaterialReference {
    MaterialReference::new(Uuid::new_v4(), Uuid::new_v4())
}

fn request(language: &str) -> SubtitleRequest {
    SubtitleRequest::new(source(), "/tmp/clip.mp4", language)
}

async fn eventually(check: impl Fn() -> bool) {
    for _ in 0..2000 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("never happened");
}

fn states(runs: &SubtitleRuns) -> Vec<SubtitleRunState> {
    runs.runs().iter().map(|run| run.state).collect()
}

fn outcomes(runs: &SubtitleRuns) -> Vec<SubtitleOutcome> {
    runs.finished().into_iter().map(|run| run.outcome).collect()
}

const WORKING: SubtitleRunState = SubtitleRunState::Working { fraction: 0.5 };

// MARK: SubtitleRunsTests

#[tokio::test]
async fn runs_one_at_a_time_and_saves_each_on_its_version() {
    let f = fixture();
    f.runs.load_availability().await;
    let (first, second) = (request("en-US"), request("en-US"));

    assert!(f.runs.start(first.clone(), Uuid::new_v4(), true));
    assert!(f.runs.start(second.clone(), Uuid::new_v4(), true));
    eventually(|| states(&f.runs).first() == Some(&WORKING)).await;
    assert_eq!(states(&f.runs), [WORKING, SubtitleRunState::Waiting]);
    f.speech.release();
    f.runs.finish().await;

    assert_eq!(f.speech.most_at_once.load(Ordering::SeqCst), 1);
    let saved = locked(&f.saved).clone();
    let of = |source: MaterialReference| {
        saved
            .iter()
            .find(|(at, _)| *at == source)
            .map(|(_, subtitles)| subtitles.clone())
    };
    assert_eq!(
        of(first.source).map(|s| s.transcript.text()).as_deref(),
        Some("Hello there.")
    );
    assert_eq!(
        of(second.source).map(|s| s.language().to_owned()).as_deref(),
        Some("en-US")
    );
    assert!(f.runs.failure().is_none());
    assert_eq!(f.runs.preferred_language().as_deref(), Some("en-US"));
}

#[tokio::test]
async fn a_card_shows_each_run_on_its_item_with_its_progress() {
    let f = fixture();
    f.runs.load_availability().await;
    let (first, second) = (request("en-US"), request("en-US"));
    assert!(f.runs.start(first.clone(), Uuid::new_v4(), true));
    assert!(f.runs.start(second.clone(), Uuid::new_v4(), true));
    eventually(|| states(&f.runs).first() == Some(&WORKING)).await;

    let rows: Vec<_> = f
        .runs
        .runs()
        .iter()
        .map(crate::subtitles::service::SubtitleRunRow::new)
        .collect();
    assert_eq!(
        rows.iter().map(|row| row.item_id.clone()).collect::<Vec<_>>(),
        [
            first.source.material_id.to_string(),
            second.source.material_id.to_string()
        ]
    );
    assert_eq!(rows[0].fraction, Some(0.5));
    assert_eq!(rows[1].fraction, None);
    assert!(rows[0].progress.contains("50%"), "{}", rows[0].progress);
    f.speech.release();
    f.runs.finish().await;
}

#[tokio::test]
async fn built_in_hears_a_language_the_transcriber_lacks_with_dictation() {
    let f = fixture();
    f.runs.load_availability().await;
    f.speech.release();

    let built_in = SubtitleEngine::BuiltIn;
    assert_eq!(f.runs.model("en-US", built_in), Some(SpeechModel::Transcriber));
    assert_eq!(f.runs.model("ru-RU", built_in), Some(SpeechModel::Dictation));
    assert!(!f.runs.start(request("lv-LV"), Uuid::new_v4(), true));
    f.runs.agree_to_model("ru-RU");
    assert!(f.runs.start(request("ru-RU"), Uuid::new_v4(), true));
    f.runs.finish().await;
    let heard = f.speech.heard();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].model, SpeechModel::Dictation);
    assert!(heard[0].may_download);
}

#[tokio::test]
async fn a_model_that_isnt_here_waits_for_the_agreement_to_its_own_language() {
    let speech = FakeSpeech::new();
    speech.edit(|available| {
        available.languages.insert(
            SpeechModel::Dictation,
            vec!["en-US".to_owned(), "ru-RU".to_owned(), "uk-UA".to_owned()],
        );
    });
    let f = fixture_with(speech);
    assert!(!f.runs.start(request("en-US"), Uuid::new_v4(), true));
    f.runs.load_availability().await;

    let built_in = SubtitleEngine::BuiltIn;
    assert!(!f.runs.needs_consent("en-US", built_in));
    assert!(f.runs.needs_consent("ru-RU", built_in));
    assert!(!f.runs.start(request("ru-RU"), Uuid::new_v4(), true));
    f.runs.agree_to_model("ru-RU");
    assert!(!f.runs.needs_consent("ru-RU", built_in) && f.runs.agreed_to_model("ru-RU"));
    assert_eq!(
        f.preferences.value("onDevice.modelConsent.speech.ru-RU"),
        Some(serde_json::Value::Bool(true))
    );
    assert!(f.runs.needs_consent("uk-UA", built_in) && !f.runs.agreed_to_model("uk-UA"));
    assert!(!f.runs.start(request("uk-UA"), Uuid::new_v4(), true));
    assert!(f.runs.start(request("ru-RU"), Uuid::new_v4(), true));
    f.speech.release();
    f.runs.finish().await;
    let languages: Vec<String> = f.speech.heard().into_iter().map(|h| h.language).collect();
    assert_eq!(languages, ["ru-RU"]);
}

#[tokio::test]
async fn a_model_found_missing_asks_for_the_agreement_though_it_was_listed() {
    let f = fixture();
    f.speech
        .set_failure(Some(PortError::Transcription(TranscriptionError::ModelNeeded)));
    f.speech.release();
    f.runs.load_availability().await;
    assert!(!f.runs.needs_consent("en-US", SubtitleEngine::BuiltIn));

    f.runs.start(request("en-US"), Uuid::new_v4(), true);
    f.runs.finish().await;
    assert!(f.runs.needs_consent("en-US", SubtitleEngine::BuiltIn));
    assert!(!f.runs.start(request("en-US"), Uuid::new_v4(), true));

    f.runs.agree_to_model("en-US");
    f.speech.set_failure(None);
    assert!(f.runs.start(request("en-US"), Uuid::new_v4(), true));
    f.runs.finish().await;
    let downloads: Vec<bool> = f.speech.heard().iter().map(|h| h.may_download).collect();
    assert_eq!(downloads, [false, true]);
}

#[tokio::test]
async fn parakeet_waits_for_its_download_not_for_the_agreement() {
    let speech = FakeSpeech::new();
    speech.edit(|available| {
        available
            .languages
            .insert(SpeechModel::ParakeetUltra, vec!["de".to_owned(), "en".to_owned()]);
    });
    let f = fixture_with(speech);
    f.runs.load_availability().await;
    let mut german = request("de-DE");
    german.engine = SubtitleEngine::Parakeet;

    let parakeet = SubtitleEngine::Parakeet;
    assert_eq!(f.runs.model("de-DE", parakeet), Some(SpeechModel::ParakeetUltra));
    assert_eq!(f.runs.model("de-DE", SubtitleEngine::BuiltIn), None);
    assert!(!f.runs.needs_consent("de-DE", parakeet));
    assert!(f.runs.needs_download("de-DE", parakeet));
    assert!(!f.runs.needs_download("en-US", SubtitleEngine::BuiltIn));
    assert!(!f.runs.start(german.clone(), Uuid::new_v4(), true));

    f.speech.edit(|available| {
        available.installed.insert(
            SpeechModel::ParakeetUltra,
            BTreeSet::from(["de".to_owned(), "en".to_owned()]),
        );
    });
    f.runs.load_availability().await;
    f.speech.release();
    assert!(f.runs.start(german, Uuid::new_v4(), true));
    f.runs.finish().await;
    let models: Vec<SpeechModel> = f.speech.heard().into_iter().map(|h| h.model).collect();
    assert_eq!(models, [SpeechModel::ParakeetUltra]);
}

#[tokio::test]
async fn a_second_run_on_the_same_item_is_refused() {
    let f = fixture();
    f.runs.load_availability().await;
    let first = request("en-US");
    let mut second = request("en-US");
    second.source = first.source;

    assert!(f.runs.start(first, Uuid::new_v4(), true));
    assert!(!f.runs.start(second.clone(), Uuid::new_v4(), true));
    f.speech.release();
    f.runs.finish().await;
    assert!(f.runs.start(second, Uuid::new_v4(), true));
    f.runs.finish().await;
}

#[tokio::test]
async fn canceling_saves_nothing() {
    let f = fixture();
    f.runs.load_availability().await;
    f.runs.start(request("en-US"), Uuid::new_v4(), true);
    f.runs.start(request("en-US"), Uuid::new_v4(), true);
    eventually(|| states(&f.runs).first() == Some(&WORKING)).await;

    let ids: Vec<Uuid> = f.runs.runs().iter().map(|run| run.id).collect();
    f.runs.cancel(ids[1]);
    f.runs.cancel(ids[0]);
    f.runs.finish().await;

    assert!(locked(&f.saved).is_empty());
    assert_eq!(f.runs.runs().len(), 0);
    assert_eq!(
        outcomes(&f.runs),
        [SubtitleOutcome::Canceled, SubtitleOutcome::Canceled]
    );
    assert!(f.runs.failure().is_none());
}

#[tokio::test]
async fn no_speech_fails_with_its_own_words() {
    let f = fixture();
    f.speech
        .set_failure(Some(PortError::Transcription(TranscriptionError::NoSpeech)));
    f.speech.release();
    f.runs.load_availability().await;

    f.runs.start(request("en-US"), Uuid::new_v4(), true);
    f.runs.finish().await;

    assert_eq!(
        f.runs.failure().map(|failure| failure.message),
        Some(text::NO_SPEECH.to_owned())
    );
    assert_eq!(
        outcomes(&f.runs),
        [SubtitleOutcome::Failed(text::NO_SPEECH.to_owned())]
    );
}

// MARK: AgentSubtitleRunsTests

struct AgentFixture {
    f: Fixture,
    agent: Arc<AgentSubtitleRuns>,
    gate: Arc<Gate>,
    source: MaterialReference,
}

fn agent_fixture() -> AgentFixture {
    let speech = FakeSpeech::new();
    speech.edit(|available| {
        available
            .languages
            .insert(SpeechModel::ParakeetUltra, vec!["de".to_owned(), "lv".to_owned()]);
    });
    let f = fixture_with(speech);
    let agent = AgentSubtitleRuns::new(Arc::clone(&f.runs), None, Arc::new(|| AgentBudgetUnit::Credits));
    AgentFixture {
        f,
        agent,
        gate: Gate::open(None),
        source: source(),
    }
}

fn command(
    source: MaterialReference,
    language: Option<&str>,
    engine: Option<AgentSubtitleEngine>,
    request_id: &str,
    fingerprint: char,
) -> SubtitlesCommand {
    SubtitlesCommand {
        action: AgentSubtitlesAction::Make,
        item: wire_id(source.material_id),
        request_id: Some(request_id.to_owned()),
        fingerprint: Some(fingerprint.to_string().repeat(64)),
        language: language.map(str::to_owned),
        engine,
        format: None,
    }
}

fn refusals() -> Refusals<AgentError> {
    Refusals {
        conflict: AgentError::request_conflict(),
        unreadable: AgentError::new(ErrorCode::FAILED, "unreadable", true, None),
        never_queued: AgentError::new(ErrorCode::FAILED, "never queued", true, None),
    }
}

type Recorded = Arc<Mutex<Vec<Uuid>>>;

impl AgentFixture {
    async fn make(&self, fields: &SubtitlesCommand, recorded: &Recorded) -> Result<AgentCreated, AgentError> {
        let gate = Arc::clone(&self.gate);
        let recorded = Arc::clone(recorded);
        let (request_id, fingerprint) = (fields.request_id.clone(), fields.fingerprint.clone());
        let target = SubtitleTarget {
            source: self.source,
            file: PathBuf::from("/tmp/talk.mp4"),
            subtitles: None,
        };
        self.agent
            .make(
                fields,
                Some("agent".to_owned()),
                &self.gate,
                refusals(),
                move |ids| {
                    locked(&recorded).extend_from_slice(ids);
                    gate.record(
                        request_id.as_deref().unwrap_or_default(),
                        fingerprint.as_deref().unwrap_or_default(),
                        None,
                        ids.to_vec(),
                        OffsetDateTime::now_utc(),
                        Some(ON_DEVICE_KIND),
                    )
                    .map_err(|_| AgentError::new(ErrorCode::FAILED, "can't record", true, None))
                },
                move || async move { Ok(target) },
            )
            .await
    }

    async fn refusal(&self, fields: &SubtitlesCommand) -> Option<AgentError> {
        self.make(fields, &Recorded::default()).await.err()
    }
}

#[tokio::test]
async fn the_same_request_starts_one_run_and_answers_with_it() {
    let a = agent_fixture();
    let recorded = Recorded::default();
    let fields = command(a.source, Some("en-US"), None, "subs-1", 'a');
    let first = a.make(&fields, &recorded).await.expect("made");
    let again = a.make(&fields, &recorded).await.expect("made again");
    assert!(!first.known && again.known);
    let ids: Vec<_> = first.jobs.iter().map(|job| job.id).collect();
    assert_eq!(ids, again.jobs.iter().map(|job| job.id).collect::<Vec<_>>());
    assert_eq!(
        ids,
        locked(&recorded)
            .iter()
            .map(|id| wire_id(*id))
            .collect::<Vec<_>>()
    );
    assert_eq!(a.f.runs.runs().len(), 1);
    let job = first.jobs[0].clone();
    assert_eq!(job.kind, "on_device");
    assert_eq!(job.operation.as_deref(), Some("subtitles"));
    assert_eq!(job.model.as_deref(), Some("speech_transcriber"));
    assert_eq!(job.item, Some(wire_id(a.source.material_id)));
    assert_eq!(job.request_id.as_deref(), Some("subs-1"));
    assert_eq!(job.client.as_deref(), Some("agent"));
    assert_eq!(job.price, Some(AgentAmount::new(0, AgentBudgetUnit::Credits)));
    assert!(job.is_price_exact);
    let other = command(a.source, Some("ru-RU"), None, "subs-1", 'b');
    assert_eq!(
        a.refusal(&other).await.map(|error| error.code),
        Some(ErrorCode::REQUEST_CONFLICT)
    );
    a.f.speech.release();
    a.f.runs.finish().await;
    let id: Uuid = job.id.into();
    assert_eq!(a.agent.job(id).map(|job| job.state).as_deref(), Some("done"));
    assert!(a.agent.is_settled(id));
    assert_eq!(
        locked(&a.f.saved)
            .first()
            .map(|(_, subtitles)| subtitles.transcript.text())
            .as_deref(),
        Some("Hello there.")
    );
    let started: Vec<Uuid> = a.agent.started().iter().map(|(_, id)| *id).collect();
    assert_eq!(started, [id]);
}

#[tokio::test]
async fn an_agents_run_leaves_the_persons_language() {
    let a = agent_fixture();
    a.f.preferences
        .set_value(
            SubtitleRuns::LAST_LANGUAGE_KEY,
            Some(serde_json::Value::from("ru-RU")),
        )
        .expect("written");
    a.make(
        &command(a.source, Some("en-US"), None, "subs-1", 'a'),
        &Recorded::default(),
    )
    .await
    .expect("made");
    a.f.speech.release();
    a.f.runs.finish().await;
    assert_eq!(a.f.speech.heard()[0].language, "en-US");
    assert_eq!(a.f.runs.preferred_language().as_deref(), Some("ru-RU"));
}

#[tokio::test]
async fn a_language_is_matched_by_its_code_and_the_engine_by_its_name() {
    let a = agent_fixture();
    a.f.speech.edit(|available| {
        available.installed.insert(
            SpeechModel::ParakeetUltra,
            BTreeSet::from(["de".to_owned(), "lv".to_owned()]),
        );
    });
    let built_in = Some(AgentSubtitleEngine::BuiltIn);
    let english = a
        .make(
            &command(a.source, Some("en"), built_in, "subs-1", 'a'),
            &Recorded::default(),
        )
        .await
        .expect("made");
    assert_eq!(english.jobs[0].model.as_deref(), Some("speech_transcriber"));
    a.f.speech.release();
    a.f.runs.finish().await;
    let parakeet = Some(AgentSubtitleEngine::Parakeet);
    let german = a
        .make(
            &command(a.source, Some("de-AT"), parakeet, "subs-2", 'a'),
            &Recorded::default(),
        )
        .await
        .expect("made");
    assert_eq!(german.jobs[0].model.as_deref(), Some("parakeet_ultra"));
    a.f.runs.finish().await;
    let languages: Vec<String> = a.f.speech.heard().into_iter().map(|h| h.language).collect();
    assert_eq!(languages, ["en-US", "de"]);
}

#[tokio::test]
async fn what_only_the_person_can_do_asks_for_them() {
    let a = agent_fixture();
    // Parakeet takes Latvian but isn't downloaded; Dictation's Russian model needs the person's agreement.
    let auto = Some(AgentSubtitleEngine::Auto);
    let latvian = a
        .refusal(&command(a.source, Some("lv"), auto, "subs-1", 'a'))
        .await
        .expect("refused");
    assert_eq!(latvian.code, ErrorCode::NEEDS_PERSON);
    assert!(
        latvian
            .next
            .as_deref()
            .is_some_and(|next| next.contains("sheet subtitles"))
    );
    assert!(latvian.message.contains("Parakeet"));
    let russian = a
        .refusal(&command(a.source, Some("ru-RU"), None, "subs-2", 'a'))
        .await
        .expect("refused");
    assert_eq!(russian.code, ErrorCode::NEEDS_PERSON);
    assert!(russian.message.contains("Hugging Face"));
    let japanese = a
        .refusal(&command(a.source, Some("ja-JP"), None, "subs-3", 'a'))
        .await;
    assert_eq!(japanese.map(|error| error.code), Some(ErrorCode::INVALID_REQUEST));
    let built_in = Some(AgentSubtitleEngine::BuiltIn);
    let built_in_latvian = a
        .refusal(&command(a.source, Some("lv"), built_in, "subs-4", 'a'))
        .await;
    assert_eq!(
        built_in_latvian.map(|error| error.code),
        Some(ErrorCode::INVALID_REQUEST)
    );
    assert_eq!(a.f.runs.runs().len(), 0);
}

#[tokio::test]
async fn a_second_run_on_the_item_is_busy_and_agents_cancel_only_their_own() {
    let a = agent_fixture();
    let made = a
        .make(
            &command(a.source, Some("en-US"), None, "subs-1", 'a'),
            &Recorded::default(),
        )
        .await
        .expect("made");
    let id: Uuid = made.jobs[0].id.into();
    let busy = a
        .refusal(&command(a.source, Some("en-US"), None, "subs-2", 'a'))
        .await;
    assert_eq!(busy.map(|error| error.code), Some(ErrorCode::BUSY));
    let personal = Uuid::new_v4();
    assert!(a.f.runs.start(request("en-US"), personal, true));
    assert!(a.agent.cancel(personal).await.is_err());
    let canceled = a.agent.cancel(id).await.expect("canceled");
    assert_eq!(canceled.map(|job| job.state).as_deref(), Some("canceled"));
    assert!(a.agent.cancel(Uuid::new_v4()).await.expect("answered").is_none());
    a.f.speech.release();
    a.f.runs.finish().await;
    assert_eq!(locked(&a.f.saved).len(), 1);
}

#[test]
fn get_and_export_read_the_versions_cues() {
    let source = source();
    let transcript = Transcript {
        language: "en-US".to_owned(),
        model: SpeechModel::Transcriber,
        revision: "1".to_owned(),
        words: vec![
            TimedWord::new("Hello", 0.2, 0.6),
            TimedWord::new("there.", 0.7, 1.1),
        ],
    };
    // As the person edited them: two lines in one cue.
    let subtitles = Subtitles::new(
        transcript,
        Some(vec![SubtitleCue::new(0.2, 1.5, "Hello\nthere.")]),
        OffsetDateTime::UNIX_EPOCH,
    );
    let srt = Some(AgentSubtitleFormat::Srt);
    let answer = AgentSubtitleRuns::answer(Some(&subtitles), source, srt, None).expect("answered");
    assert_eq!(answer.item, wire_id(source.material_id));
    assert_eq!(answer.version_id, wire_id(source.version_id));
    let cue = AgentCue {
        start: 0.2,
        end: 1.5,
        text: "Hello\nthere.".to_owned(),
    };
    assert_eq!(answer.cues, [cue]);
    assert_eq!(
        answer.text.as_deref(),
        Some("1\n00:00:00,200 --> 00:00:01,500\nHello\nthere.\n")
    );
    assert!(answer.file.is_none());
    let plain = AgentSubtitleRuns::answer(Some(&subtitles), source, None, None).expect("answered");
    assert!(plain.text.is_none());
    assert!(AgentSubtitleRuns::answer(None, source, None, None).is_err());
    let name = AgentSubtitleRuns::file_name(
        Path::new("/tmp/Talk take.mp4"),
        "en-US",
        AgentSubtitleFormat::Vtt,
        &BTreeSet::from(["talk take.en-us.vtt".to_owned()]),
    );
    assert_ne!(name, "Talk take.en-US.vtt");
    assert!(name.starts_with("Talk take"));
    assert_eq!(Path::new(&name).extension(), Some("vtt".as_ref()));
}

#[tokio::test]
async fn capabilities_list_each_engines_languages_and_model() {
    let a = agent_fixture();
    a.f.runs.load_availability().await;
    let capabilities = a.agent.capabilities();
    assert_eq!(capabilities.default_engine, "auto");
    let engines: Vec<&str> = capabilities
        .engines
        .iter()
        .map(|engine| engine.engine.as_str())
        .collect();
    assert_eq!(engines, ["built_in", "parakeet"]);
    assert_eq!(capabilities.engines[0].languages, ["en-US", "ru-RU"]);
    assert_eq!(capabilities.engines[0].model, "needs_consent");
    assert_eq!(
        capabilities.engines[0].needs_consent,
        Some(vec!["ru-RU".to_owned()])
    );
    assert_eq!(capabilities.engines[1].languages, ["de", "lv"]);
    assert_eq!(capabilities.engines[1].model, "not_downloaded");
    a.f.runs.agree_to_model("ru-RU");
    a.f.runs.set_default_engine(SubtitleEngine::Parakeet);
    let capabilities = a.agent.capabilities();
    assert_eq!(capabilities.engines[0].model, "agreed");
    assert_eq!(capabilities.default_engine, "parakeet");
    assert_eq!(capabilities.engines[0].needs_consent, Some(Vec::new()));
    a.f.speech.release();
}

// MARK: SpeechModelsTests

/// Keeps "downloaded" in memory: a download completes, or fails with `failure`.
#[derive(Default)]
struct FakeModelDownloader {
    complete: AtomicBool,
    failure: Mutex<Option<PortError>>,
}

fn is_current(folder: &Path) -> bool {
    folder
        .file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(&WhisperModel::REVISION[..8]))
}

#[async_trait]
impl SpeechModelDownloading for FakeModelDownloader {
    fn is_complete(&self, _manifest: &ModelManifest, _folder: &Path) -> bool {
        self.complete.load(Ordering::SeqCst)
    }

    async fn download(
        &self,
        _manifest: &ModelManifest,
        _folder: &Path,
        progress: Progress<f64>,
    ) -> Result<(), PortError> {
        progress(0.5);
        if let Some(failure) = locked(&self.failure).clone() {
            return Err(failure);
        }
        self.complete.store(true, Ordering::SeqCst);
        progress(1.0);
        Ok(())
    }

    /// The current revision's folder takes 1000 bytes once downloaded; another folder takes what is in it.
    fn size(&self, folder: &Path) -> i64 {
        if is_current(folder) {
            return if self.complete.load(Ordering::SeqCst) {
                1000
            } else {
                0
            };
        }
        std::fs::read_dir(folder).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.metadata().ok())
                .map(|metadata| i64::try_from(metadata.len()).unwrap_or(0))
                .sum()
        })
    }

    fn remove(&self, folder: &Path) -> Result<(), PortError> {
        let _ = std::fs::remove_dir_all(folder);
        if is_current(folder) {
            self.complete.store(false, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[tokio::test]
async fn downloads_into_a_folder_of_its_revision_and_removes() {
    let downloader = Arc::new(FakeModelDownloader::default());
    let models = SpeechModels::new(Path::new("/tmp/speech-models"), downloader);
    let changes = models.subscribe();
    assert_eq!(models.state(), SpeechModelState::NotDownloaded);
    assert_eq!(
        models
            .folder()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        Some(format!("whisper-small-q5_1-{}", &WhisperModel::REVISION[..8]))
    );

    models.download();
    assert!(models.is_downloading());
    models.finish().await;
    assert_eq!(models.state(), SpeechModelState::Ready);
    assert_eq!(models.size(), 1000);
    assert!(changes.has_changed().unwrap_or(false));

    models.remove();
    assert_eq!(models.state(), SpeechModelState::NotDownloaded);
    assert_eq!(models.size(), 0);
}

#[tokio::test]
async fn a_failed_download_says_so() {
    let downloader = Arc::new(FakeModelDownloader::default());
    *locked(&downloader.failure) = Some(PortError::Other(
        "The Internet connection appears to be offline.".to_owned(),
    ));
    let models = SpeechModels::new(Path::new("/tmp/speech-models"), downloader);

    models.download();
    models.finish().await;

    assert_eq!(models.state(), SpeechModelState::NotDownloaded);
    assert_eq!(
        models.failure().map(|failure| failure.title),
        Some(text::DOWNLOAD_FAILED_TITLE.to_owned())
    );
}

#[tokio::test]
async fn a_canceled_download_says_nothing() {
    let downloader = Arc::new(FakeModelDownloader::default());
    *locked(&downloader.failure) = Some(PortError::Cancelled);
    let models = SpeechModels::new(Path::new("/tmp/speech-models"), downloader);

    models.download();
    models.cancel();
    models.finish().await;

    assert_eq!(models.state(), SpeechModelState::NotDownloaded);
    assert!(models.failure().is_none());
}

#[tokio::test]
async fn an_earlier_revision_counts_until_the_current_one_is_downloaded() {
    let root = tempfile::tempdir().expect("a temp dir");
    let old = root.path().join("whisper-small-q5_1-00000000");
    let other = root.path().join("other-model");
    for folder in [&old, &other] {
        std::fs::create_dir_all(folder).expect("a folder");
        std::fs::write(folder.join("weights.bin"), vec![0_u8; 300]).expect("written");
    }

    let models = SpeechModels::new(root.path(), Arc::new(FakeModelDownloader::default()));
    assert_eq!(models.size(), 300);

    models.download();
    models.finish().await;
    assert_eq!(models.size(), 1000);
    assert!(!old.exists());
    assert!(other.exists());
}

#[test]
fn removing_also_removes_earlier_revisions() {
    let root = tempfile::tempdir().expect("a temp dir");
    let old = root.path().join("whisper-small-q5_1-00000000");
    std::fs::create_dir_all(&old).expect("a folder");
    std::fs::write(old.join("weights.bin"), vec![0_u8; 300]).expect("written");

    let models = SpeechModels::new(root.path(), Arc::new(FakeModelDownloader::default()));
    models.remove();
    assert_eq!(models.size(), 0);
    assert!(!old.exists());
}

// MARK: SubtitleEditor

/// `SubtitleEditor.save`: Done saves onto the version the editor opened on, found among the item's versions, though
/// a newer version came since; only a version that is gone isn't saved.
#[tokio::test]
async fn done_saves_onto_the_opened_version_after_a_newer_one() {
    let f = fixture();
    let store = Arc::new(jutsu_store::Store::open_in_memory().await.expect("a store"));
    let agent = AgentSubtitleRuns::new(Arc::clone(&f.runs), None, Arc::new(|| AgentBudgetUnit::Credits));
    let models = SpeechModels::new(
        Path::new("/tmp/speech-models"),
        Arc::new(FakeModelDownloader::default()),
    );
    let service = SubtitleService::new(Arc::clone(&store), Arc::clone(&f.runs), models, agent);
    let opened = Subtitles::new(
        Transcript {
            language: "en-US".to_owned(),
            model: SpeechModel::Transcriber,
            revision: "1".to_owned(),
            words: vec![TimedWord::new("Hello", 0.2, 0.6)],
        },
        None,
        OffsetDateTime::UNIX_EPOCH,
    );
    let mut material = crate::library::fixture::video(1.0);
    material.current.subtitles = Some(opened.clone());
    let opened_on = material.reference();
    let mut newer = material.current.clone();
    newer.id = Uuid::new_v4();
    newer.subtitles = None;
    material.add_version(newer).expect("a newer version");
    store.save_material(material).await.expect("saved");

    let mut edited = opened.clone();
    edited.cues = vec![SubtitleCue::new(0.2, 1.5, "Hello again")];
    assert_eq!(
        service.save(opened_on, &opened, edited.clone(), false).await,
        SubtitleSaveOutcome::Saved
    );
    let stored = store
        .material(opened_on.material_id)
        .await
        .expect("read")
        .expect("there");
    let saved = stored
        .versions()
        .into_iter()
        .find(|version| version.id == opened_on.version_id)
        .and_then(|version| version.subtitles.clone());
    assert_eq!(saved, Some(edited.clone()));
    assert!(
        stored.current.subtitles.is_none(),
        "the newer version keeps its own"
    );

    let gone = MaterialReference::new(opened_on.material_id, Uuid::new_v4());
    let SubtitleSaveOutcome::Failed { message } = service.save(gone, &opened, edited, false).await else {
        panic!("a version that is gone isn't saved");
    };
    assert_eq!(message.message, text::VERSION_GONE_MESSAGE);
}
