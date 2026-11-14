//! An agent's `subtitles` (`AgentSubtitleRuns.swift`): make starts a `SubtitleRuns` run by the sheet's rules, once for
//! its request ID through the request gate, followed with `jobs` as kind `on_device` like enhance on this computer;
//! get and export read the current version's cues. Free, so no `max_cost`, budget or hold applies. Which runs agents
//! started is kept in memory: a relaunch forgets them.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentCreated, AgentCue, AgentError, AgentJob, AgentSubtitleCapabilities,
    AgentSubtitleEngine, AgentSubtitleEngineState, AgentSubtitleFormat, AgentSubtitles, ErrorCode,
    SubtitlesCommand, Timestamp,
};
use jutsu_core::{
    MaterialReference, OutputNamePattern, SpeechAvailability, SpeechModel, SpeechSelection, SubtitleEngine,
    SubtitleFormat, Subtitles,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::models::{SpeechModelState, SpeechModels};
use super::runs::{SubtitleOutcome, SubtitleRequest, SubtitleRunState, SubtitleRuns};
use crate::agents::request_gate::{AgentRaiseOutcome, AgentRequestGate, Refusals};
use crate::agents::{app_id, wire_id};

/// What `jobs` calls a run on this computer (`AgentOnDeviceRuns.kind`).
pub const ON_DEVICE_KIND: &str = "on_device";
/// What `jobs` calls a subtitles run's operation.
pub const OPERATION: &str = "subtitles";

/// A run on this computer a relaunch forgot, answered as enhance answers its request (`AgentOnDeviceRuns.forgotten`).
pub fn forgotten(id: Uuid) -> AgentJob {
    let mut job = AgentJob::new(wire_id(id), "unknown", None);
    job.error = Some("Jutsu Studio forgets a run on this computer when it quits.".to_owned());
    ON_DEVICE_KIND.clone_into(&mut job.kind);
    job.next = Some(
        "If it finished, its result is on the item's current version: call library_get, or subtitles get, with the \
         item's ID. Otherwise send the request again with a new request_id."
            .to_owned(),
    );
    job
}

/// The video an agent named: its current version, its file, and the subtitles that version has.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleTarget {
    pub source: MaterialReference,
    pub file: std::path::PathBuf,
    pub subtitles: Option<Subtitles>,
}

/// The request gate `make` runs through.
pub type Gate = AgentRequestGate<AgentCreated, AgentError>;
type NoRaise = fn(Vec<Uuid>) -> futures::future::Ready<Result<AgentRaiseOutcome<AgentCreated>, AgentError>>;

#[derive(Debug, Clone)]
struct Record {
    request_id: String,
    client: Option<String>,
}

/// The runs agents started, and what they read.
pub struct AgentSubtitleRuns {
    runs: Arc<SubtitleRuns>,
    models: Option<Arc<SpeechModels>>,
    price_unit: Arc<dyn Fn() -> AgentBudgetUnit + Send + Sync>,
    records: Mutex<HashMap<Uuid, Record>>,
}

impl std::fmt::Debug for AgentSubtitleRuns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentSubtitleRuns")
            .finish_non_exhaustive()
    }
}

fn engine(engine: AgentSubtitleEngine) -> SubtitleEngine {
    match engine {
        AgentSubtitleEngine::Auto => SubtitleEngine::Auto,
        AgentSubtitleEngine::BuiltIn => SubtitleEngine::BuiltIn,
        AgentSubtitleEngine::Parakeet => SubtitleEngine::Parakeet,
    }
}

impl AgentSubtitleRuns {
    /// `models` is the model's download, for `capabilities`; `price_unit` the unit the free price is given in: the
    /// connection's, else cents.
    pub fn new(
        runs: Arc<SubtitleRuns>,
        models: Option<Arc<SpeechModels>>,
        price_unit: Arc<dyn Fn() -> AgentBudgetUnit + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runs,
            models,
            price_unit,
            records: Mutex::new(HashMap::new()),
        })
    }

    fn records(&self) -> MutexGuard<'_, HashMap<Uuid, Record>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the run once for its request ID: the same request again answers with the run it started. `record`
    /// keeps the request with the run's ID before it starts; `target` reads the item.
    #[allow(clippy::too_many_arguments, reason = "the Swift method's parameters")]
    pub async fn make<R, T, TF>(
        self: &Arc<Self>,
        fields: &SubtitlesCommand,
        client: Option<String>,
        gate: &Arc<Gate>,
        refusals: Refusals<AgentError>,
        record: R,
        target: T,
    ) -> Result<AgentCreated, AgentError>
    where
        R: FnOnce(&[Uuid]) -> Result<(), AgentError> + Send + 'static,
        T: FnOnce() -> TF + Send + 'static,
        TF: Future<Output = Result<SubtitleTarget, AgentError>> + Send + 'static,
    {
        let (Some(request_id), Some(fingerprint)) = (fields.request_id.clone(), fields.fingerprint.clone())
        else {
            return Err(AgentError::bad_request("make needs request_id"));
        };
        let me = Arc::clone(self);
        let fields = fields.clone();
        let queued_id = request_id.clone();
        let known_id = request_id.clone();
        gate.run(
            &request_id,
            &fingerprint,
            None,
            refusals,
            || {},
            |ids| async move {
                let jobs: Vec<AgentJob> = ids.iter().filter_map(|id| self.job(*id)).collect();
                Ok((!jobs.is_empty()).then(|| AgentCreated::new(&known_id, true, jobs)))
            },
            None::<NoRaise>,
            move || async move {
                if me.runs.availability().is_none() {
                    me.runs.load_availability().await;
                }
                let target = target().await?;
                me.start(&fields, &queued_id, client, &target, record)
            },
        )
        .await
    }

    /// Starts the run as the sheet would, once `record` kept the request with the run's ID; the request gate around
    /// it answers the same request again with the run it started.
    pub fn start(
        &self,
        fields: &SubtitlesCommand,
        request_id: &str,
        client: Option<String>,
        target: &SubtitleTarget,
        record: impl FnOnce(&[Uuid]) -> Result<(), AgentError>,
    ) -> Result<AgentCreated, AgentError> {
        let engine = fields.engine.map_or_else(|| self.runs.default_engine(), engine);
        let language = self.language(fields.language.as_deref(), engine)?;
        let item = target.source.material_id;
        if let Some(refusal) = self.refusal(&language, engine, item) {
            return Err(refusal);
        }
        let id = Uuid::new_v4();
        record(&[id])?;
        let request = SubtitleRequest {
            source: target.source,
            file: target.file.clone(),
            language: language.clone(),
            engine,
        };
        // An agent's language isn't the person's: the sheet keeps starting on theirs.
        if !self.runs.start(request, id, false) {
            return Err(self
                .refusal(&language, engine, item)
                .unwrap_or_else(|| AgentError::new(ErrorCode::FAILED, "The run didn't start.", true, None)));
        }
        self.records().insert(
            id,
            Record {
                request_id: request_id.to_owned(),
                client,
            },
        );
        Ok(AgentCreated::new(
            request_id,
            false,
            vec![self.job(id).unwrap_or_else(|| forgotten(id))],
        ))
    }

    /// The language as the sheet offers it: the one asked for, matched exactly and then by its code ("en" takes
    /// "en-US"), else the person's; refused when no model here takes it.
    fn language(&self, asked: Option<&str>, engine: SubtitleEngine) -> Result<String, AgentError> {
        let offered = self.runs.languages();
        let asked = asked
            .map(str::to_owned)
            .or_else(|| self.runs.preferred_language())
            .unwrap_or_default();
        let code = SpeechSelection::language_code(&asked);
        let matched = offered
            .iter()
            .find(|language| language.to_lowercase() == asked.to_lowercase())
            .or_else(|| {
                offered.iter().find(|language| {
                    self.runs.model(language, engine).is_some()
                        && SpeechSelection::language_code(language) == code
                })
            })
            .or_else(|| {
                offered
                    .iter()
                    .find(|language| SpeechSelection::language_code(language) == code)
            });
        matched.cloned().ok_or_else(|| {
            AgentError::new(
                ErrorCode::INVALID_REQUEST,
                format!(
                    "No speech model on this computer takes {}.",
                    asked.chars().take(35).collect::<String>()
                ),
                false,
                Some("Pick one of capabilities' onDevice.subtitles languages."),
            )
        })
    }

    /// Why the sheet wouldn't start the run now, in the order an agent can act on: the engine and language, a run
    /// already on the item, and last what only the person can do.
    fn refusal(&self, language: &str, engine: SubtitleEngine, item: Uuid) -> Option<AgentError> {
        if self.runs.model(language, engine).is_none() {
            return Some(AgentError::new(
                ErrorCode::INVALID_REQUEST,
                format!("{} doesn't take {language}.", engine.raw_value()),
                false,
                Some("Use engine auto, or one whose languages in capabilities' onDevice.subtitles list it."),
            ));
        }
        if let Some(run) = self.runs.run_for(item) {
            let run_id = wire_id(run.id);
            return Some(AgentError::new(
                ErrorCode::BUSY,
                format!(
                    "Item {} already has subtitles being made, {run_id}.",
                    wire_id(item)
                ),
                true,
                Some(&format!(
                    "Follow it with jobs (ids [{run_id}]) and send make again once it is over, or cancel it with \
                     job_action if an agent started it."
                )),
            ));
        }
        let show = format!(
            "Call show with item {} and sheet subtitles, and tell the person; ",
            wire_id(item)
        );
        if self.runs.needs_download(language, engine) {
            return Some(AgentError::new(
                ErrorCode::NEEDS_PERSON,
                format!(
                    "Nothing was started: {language} needs Parakeet Ultra, which isn't downloaded, and only the person \
                     downloads it."
                ),
                true,
                Some(&format!(
                    "{show}once they downloaded it in Settings › Subtitles, send make again with a new request_id."
                )),
            ));
        }
        if self.runs.needs_consent(language, engine) {
            return Some(AgentError::new(
                ErrorCode::NEEDS_PERSON,
                format!(
                    "Nothing was started: {language} needs Jutsu Studio to download its speech model from Hugging \
                     Face once, and only the person can agree to that."
                ),
                true,
                Some(&format!(
                    "{show}once they agreed (and may make the subtitles there), send make again with a new \
                     request_id."
                )),
            ));
        }
        None
    }

    // MARK: Following

    /// The runs agents started, for `jobs` without IDs.
    pub fn started(&self) -> Vec<(OffsetDateTime, Uuid)> {
        let ids: Vec<Uuid> = self.records().keys().copied().collect();
        ids.into_iter()
            .filter_map(|id| self.runs.run(id).map(|(run, _)| (run.created_at, id)))
            .collect()
    }

    /// Whether `id` is a subtitles run this computer still knows, waiting, running or over.
    pub fn knows(&self, id: Uuid) -> bool {
        self.runs.run(id).is_some()
    }

    /// Over, or not known: what `jobs`' wait waits for.
    pub fn is_settled(&self, id: Uuid) -> bool {
        self.runs.run(id).is_none_or(|(_, finished)| finished.is_some())
    }

    /// The run as `jobs` lists it: an agent's or the person's, while this computer knows it.
    pub fn job(&self, id: Uuid) -> Option<AgentJob> {
        let (run, finished) = self.runs.run(id)?;
        let record = self.records().get(&id).cloned();
        let mut job = AgentJob::new(wire_id(id), "processing", Some(Timestamp(run.created_at)));
        job.price = Some(AgentAmount::new(0, (self.price_unit)()));
        job.is_price_exact = true;
        ON_DEVICE_KIND.clone_into(&mut job.kind);
        job.request_id = record.as_ref().map(|record| record.request_id.clone());
        job.client = record.and_then(|record| record.client);
        job.model = Some(run.model.raw_value().to_owned());
        job.operation = Some(OPERATION.to_owned());
        job.item = Some(wire_id(run.request.source.material_id));
        job.finished_at = finished.as_ref().map(|finished| Timestamp(finished.finished_at));
        match (finished.map(|finished| finished.outcome), run.state) {
            (Some(SubtitleOutcome::Done(_)), _) => {
                "done".clone_into(&mut job.state);
                job.progress = Some(1.0);
                job.next = Some(
                    "subtitles get with the item reads the cues; export writes them as SRT or VTT."
                        .to_owned(),
                );
            }
            (Some(SubtitleOutcome::Failed(message)), _) => {
                "failed".clone_into(&mut job.state);
                job.error = Some(message);
            }
            (Some(SubtitleOutcome::Canceled), _) => "canceled".clone_into(&mut job.state),
            (None, SubtitleRunState::Waiting) => {
                "queued".clone_into(&mut job.state);
                job.next = Some(
                    "Subtitles runs go one at a time; it starts when the one before it is done.".to_owned(),
                );
            }
            (None, SubtitleRunState::DownloadingModel { fraction }) => {
                "downloading".clone_into(&mut job.state);
                job.progress = Some(fraction);
                job.next = Some(
                    "Jutsu Studio is downloading the language's speech model once; the words follow."
                        .to_owned(),
                );
            }
            (None, SubtitleRunState::Working { fraction }) => job.progress = Some(fraction),
        }
        Some(job)
    }

    /// Cancels a run an agent started and answers with it as it is now; `None` when `id` isn't a subtitles run.
    pub async fn cancel(&self, id: Uuid) -> Result<Option<AgentJob>, AgentError> {
        let Some((_, finished)) = self.runs.run(id) else {
            return Ok(None);
        };
        if !self.records().contains_key(&id) {
            return Err(AgentError::new(
                ErrorCode::NEEDS_PERSON,
                "The person started these subtitles; only they can cancel them.",
                false,
                Some("Ask the person, or leave it be."),
            ));
        }
        if finished.is_none() {
            self.runs.cancel(id);
            self.runs.over(id).await;
        }
        Ok(self.job(id))
    }

    // MARK: Reading

    /// get and export: the version's subtitles, with the file's text for `format`.
    pub fn answer(
        subtitles: Option<&Subtitles>,
        source: MaterialReference,
        format: Option<AgentSubtitleFormat>,
        file: Option<&Path>,
    ) -> Result<AgentSubtitles, AgentError> {
        let Some(subtitles) = subtitles else {
            return Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "The current version of {} has no subtitles.",
                    wire_id(source.material_id)
                ),
                false,
                Some("Make them with subtitles make; a new version starts without any."),
            ));
        };
        Ok(AgentSubtitles {
            item: wire_id(source.material_id),
            version_id: wire_id(source.version_id),
            language: subtitles.language().to_owned(),
            model: subtitles.transcript.model.raw_value().to_owned(),
            cues: subtitles
                .cues
                .iter()
                .map(|cue| AgentCue {
                    start: cue.start,
                    end: cue.end,
                    text: cue.lines().join("\n"),
                })
                .collect(),
            text: if file.is_none() {
                format.map(|format| Self::text(subtitles, format))
            } else {
                None
            },
            file: file.map(|file| file.to_string_lossy().into_owned()),
        })
    }

    /// The file's text, as Export for Editing writes it.
    pub fn text(subtitles: &Subtitles, format: AgentSubtitleFormat) -> String {
        let format = match format {
            AgentSubtitleFormat::Srt => SubtitleFormat::Srt,
            AgentSubtitleFormat::Vtt => SubtitleFormat::Vtt,
        };
        format.text(&subtitles.cues)
    }

    /// The export's file name: the video's own name with the language, never one `existing` has.
    pub fn file_name(
        video: &Path,
        language: &str,
        format: AgentSubtitleFormat,
        existing: &BTreeSet<String>,
    ) -> String {
        let stem = video
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = format!("{stem}.{language}.{}", format.as_str());
        OutputNamePattern::available(&name, |candidate| existing.contains(&candidate.to_lowercase()))
    }

    // MARK: Capabilities

    /// What `capabilities` says of subtitles on this computer; reads what `load_availability` last read.
    pub fn capabilities(&self) -> AgentSubtitleCapabilities {
        let availability = self.runs.availability().unwrap_or_default();
        let built_in_only = SpeechAvailability {
            languages: availability
                .languages
                .iter()
                .filter(|(model, _)| **model != SpeechModel::ParakeetUltra)
                .map(|(model, languages)| (*model, languages.clone()))
                .collect(),
            installed: std::collections::BTreeMap::new(),
        };
        let built_in = SpeechSelection::languages(&built_in_only);
        let parakeet = availability
            .languages
            .get(&SpeechModel::ParakeetUltra)
            .cloned()
            .unwrap_or_default();
        let needs_consent: Vec<String> = built_in
            .iter()
            .filter(|language| self.runs.needs_consent(language, SubtitleEngine::BuiltIn))
            .cloned()
            .collect();
        let parakeet_model = match self.models.as_ref().map(|models| models.state()) {
            Some(SpeechModelState::Ready) => "ready",
            Some(SpeechModelState::Downloading { .. }) => "downloading",
            _ => "not_downloaded",
        };
        AgentSubtitleCapabilities {
            default_engine: self.runs.default_engine().raw_value().to_owned(),
            engines: vec![
                AgentSubtitleEngineState {
                    engine: AgentSubtitleEngine::BuiltIn.as_str().to_owned(),
                    available: !built_in.is_empty(),
                    model: if needs_consent.is_empty() {
                        "agreed"
                    } else {
                        "needs_consent"
                    }
                    .to_owned(),
                    languages: built_in,
                    needs_consent: Some(needs_consent),
                },
                AgentSubtitleEngineState {
                    engine: AgentSubtitleEngine::Parakeet.as_str().to_owned(),
                    available: !parakeet.is_empty(),
                    languages: parakeet,
                    model: parakeet_model.to_owned(),
                    needs_consent: None,
                },
            ],
        }
    }
}

/// An item ID from the wire, for the runs.
pub fn item_id(fields: &SubtitlesCommand) -> Uuid {
    app_id(fields.item)
}
