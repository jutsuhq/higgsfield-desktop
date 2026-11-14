//! What the window shows of subtitles (`AppEnvironment+Subtitles.swift`, `SubtitlesSheet.swift`,
//! `SubtitleEditor.swift`, `SubtitleSettingsView.swift`): the sheet that makes them, the result page's section, the
//! editor with Done's save, the export of a file, and Settings › Subtitles with the model's download.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jutsu_core::{MaterialReference, SubtitleCue, SubtitleEngine, SubtitleFormat, Subtitles, WhisperModel};
use jutsu_store::Store;
use serde::Serialize;
use tokio::sync::watch;
use uuid::Uuid;

use super::agent::{AgentSubtitleRuns, SubtitleTarget};
use super::models::{SpeechModelState, SpeechModels};
use super::runs::{SubtitleRequest, SubtitleRuns};
use super::save::SubtitleSave;
use super::text;
use crate::connection::UserMessage;
use crate::library::media::{MediaRow, material_row};

/// One choice of a picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleOption {
    pub value: String,
    pub title: String,
}

/// A run on the item, waiting or going.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleRunView {
    pub id: String,
    /// "Making subtitles · 42%".
    pub progress: String,
    /// 0…1, `None` while it waits.
    pub fraction: Option<f64>,
}

/// A run as a Library card shows it over its frame (`OnDeviceRunBadge(run: SubtitleRuns.Run)`).
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleRunRow {
    pub id: String,
    /// The library item whose subtitles it makes.
    pub item_id: String,
    /// "Making subtitles · 42%".
    pub progress: String,
    /// 0…1, `None` while it waits.
    pub fraction: Option<f64>,
}

impl SubtitleRunRow {
    pub fn new(run: &super::runs::SubtitleRun) -> Self {
        Self {
            id: run.id.to_string(),
            item_id: run.request.source.material_id.to_string(),
            progress: text::progress(run.state),
            fraction: run.state.fraction(),
        }
    }
}

/// The Subtitles sheet and the result page's section.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSheetView {
    pub title: String,
    pub version_note: String,
    pub sound_note: String,
    pub price: String,
    pub languages: Vec<SubtitleOption>,
    /// The language picked, else the one the person used last, else the system's.
    pub language: Option<String>,
    pub engines: Vec<SubtitleOption>,
    pub engine: SubtitleEngine,
    /// Before the first run in a language whose model isn't here: the download's notice and its toggle's title.
    pub model_notice: Option<String>,
    pub model_consent: String,
    pub run: Option<SubtitleRunView>,
    /// "English (US) · 24 cues" of the version's subtitles.
    pub current: Option<String>,
    /// No model here hears speech: why the sheet can't start.
    pub unavailable: Option<String>,
    pub failure: Option<UserMessage>,
}

/// One cue as the editor lists it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCueView {
    pub id: String,
    /// "0:01.2 – 0:03.4".
    pub time: String,
    pub can_merge: bool,
    pub can_split: bool,
}

/// The editor's cues as they are now.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleEditorView {
    pub material_id: String,
    pub version_id: String,
    /// "Subtitles", the editor's heading (`SubtitleText.title`).
    pub title: String,
    pub summary: String,
    /// The subtitles as edited; the editor sends them back with each change and on Done.
    pub subtitles: Subtitles,
    pub cues: Vec<SubtitleCueView>,
}

/// One change the editor makes to its cues.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SubtitleEdit {
    SetText { id: String, text: String },
    MergeWithNext { id: String },
    Split { id: String },
}

impl SubtitleEdit {
    /// `subtitles` with the change made; an ID that isn't a cue's changes nothing.
    pub fn apply(&self, mut subtitles: Subtitles) -> Subtitles {
        let id = |id: &str| Uuid::parse_str(id).unwrap_or_default();
        match self {
            Self::SetText { id: cue, text } => subtitles.set_text(text, id(cue)),
            Self::MergeWithNext { id: cue } => subtitles.merge_with_next(id(cue)),
            Self::Split { id: cue } => subtitles.split(id(cue)),
        }
        subtitles
    }
}

/// What Done did.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SubtitleSaveOutcome {
    Saved,
    /// Made again while the editor was open: ask, then Done again with `replace`.
    ChangedMeanwhile {
        title: String,
        message: String,
    },
    Failed {
        message: UserMessage,
    },
}

/// Settings › Subtitles.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSettingsView {
    pub engines: Vec<SubtitleOption>,
    pub default_engine: SubtitleEngine,
    pub auto_note: String,
    pub models_folder: String,
    pub model_name: String,
    pub model_summary: String,
    pub model_credit: String,
    pub model_facts: String,
    pub model_languages: String,
    pub model_url: String,
    pub license: String,
    pub license_url: String,
    pub model_state: SpeechModelState,
    /// "190 MB", the folders of earlier revisions included; `None` when nothing is on disk.
    pub model_size: Option<String>,
    /// Whether this build hears speech: whisper.cpp comes with the app's `whisper` feature and needs ffmpeg.
    pub is_available: bool,
    pub failure: Option<UserMessage>,
}

/// The subtitles of the window and of agents.
pub struct SubtitleService {
    store: Arc<Store>,
    runs: Arc<SubtitleRuns>,
    models: Arc<SpeechModels>,
    agent: Arc<AgentSubtitleRuns>,
}

impl std::fmt::Debug for SubtitleService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SubtitleService").finish_non_exhaustive()
    }
}

fn engine_options() -> Vec<SubtitleOption> {
    text::OFFERED_ENGINES
        .iter()
        .map(|&engine| SubtitleOption {
            value: engine.raw_value().to_owned(),
            title: text::engine_title(engine).to_owned(),
        })
        .collect()
}

fn not_found() -> UserMessage {
    UserMessage::new(text::FAILURE_TITLE, text::CANT_OPEN_FILE, None)
}

impl SubtitleService {
    pub fn new(
        store: Arc<Store>,
        runs: Arc<SubtitleRuns>,
        models: Arc<SpeechModels>,
        agent: Arc<AgentSubtitleRuns>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            runs,
            models,
            agent,
        })
    }

    pub fn runs(&self) -> &Arc<SubtitleRuns> {
        &self.runs
    }

    pub fn models(&self) -> &Arc<SpeechModels> {
        &self.models
    }

    pub fn agent(&self) -> &Arc<AgentSubtitleRuns> {
        &self.agent
    }

    /// A revision that goes up whenever a run, the model or the languages change.
    pub fn subscribe(&self) -> (watch::Receiver<u64>, watch::Receiver<u64>) {
        (self.runs.subscribe(), self.models.subscribe())
    }

    /// Reads the languages, at launch and after the model came or went.
    pub async fn refresh(&self) {
        self.models.refresh();
        self.runs.load_availability().await;
    }

    /// The video of `material_id` with its file on this computer: its current version and that version's subtitles.
    pub async fn target(&self, material_id: Uuid) -> Option<SubtitleTarget> {
        let material = self.store.material(material_id).await.ok()??;
        let file = match material_row(&material)? {
            MediaRow::Video(item) if item.history_item.is_none() => item.output_file()?,
            _ => return None,
        };
        Some(SubtitleTarget {
            source: material.reference(),
            file,
            subtitles: material.current.subtitles.clone(),
        })
    }

    // MARK: Sheet

    pub async fn sheet(
        &self,
        material_id: Uuid,
        language: Option<String>,
        engine: Option<SubtitleEngine>,
    ) -> SubtitleSheetView {
        if self.runs.availability().is_none() {
            self.runs.load_availability().await;
        }
        let target = self.target(material_id).await;
        let engine = engine.unwrap_or_else(|| self.runs.default_engine());
        let offered = self.runs.languages();
        let language = language
            .filter(|language| offered.contains(language))
            .or_else(|| self.runs.preferred_language());
        let model_notice = language
            .as_deref()
            .filter(|language| self.runs.needs_consent(language, engine))
            .map(|_| text::model_notice());
        let run = self.runs.run_for(material_id).map(|run| SubtitleRunView {
            id: run.id.to_string(),
            progress: text::progress(run.state),
            fraction: run.state.fraction(),
        });
        SubtitleSheetView {
            title: text::TITLE.to_owned(),
            version_note: text::VERSION_NOTE.to_owned(),
            sound_note: text::SOUND_NOTE.to_owned(),
            price: text::PRICE.to_owned(),
            languages: offered
                .iter()
                .map(|language| SubtitleOption {
                    value: language.clone(),
                    title: text::language_name(language),
                })
                .collect(),
            language,
            engines: engine_options(),
            engine,
            model_notice,
            model_consent: text::MODEL_CONSENT.to_owned(),
            run,
            current: target
                .as_ref()
                .and_then(|target| target.subtitles.as_ref())
                .map(text::summary),
            unavailable: offered.is_empty().then(|| text::UNSUPPORTED_LANGUAGE.to_owned()),
            failure: self.runs.failure(),
        }
    }

    /// The sheet's Make: agrees to the model's download when the person ticked it, then starts the run.
    pub async fn start(
        &self,
        material_id: Uuid,
        language: &str,
        engine: SubtitleEngine,
        agrees_to_model: bool,
    ) -> Result<String, UserMessage> {
        let target = self.target(material_id).await.ok_or_else(not_found)?;
        if agrees_to_model {
            self.runs.agree_to_model(language);
        }
        let request = SubtitleRequest {
            source: target.source,
            file: target.file,
            language: language.to_owned(),
            engine,
        };
        let id = Uuid::new_v4();
        if self.runs.start(request, id, true) {
            return Ok(id.to_string());
        }
        let reason = if self.runs.model(language, engine).is_none() {
            text::UNSUPPORTED_LANGUAGE
        } else if self.runs.needs_consent(language, engine) {
            text::MODEL_NEEDED
        } else {
            text::MAKING
        };
        Err(UserMessage::new(text::FAILURE_TITLE, reason, None))
    }

    pub fn cancel(&self, run_id: Uuid) {
        self.runs.cancel(run_id);
    }

    /// The runs waiting or running, for the Library's cards.
    pub fn run_rows(&self) -> Vec<SubtitleRunRow> {
        self.runs.runs().iter().map(SubtitleRunRow::new).collect()
    }

    /// The failure the window shows once.
    pub fn take_failure(&self) -> Option<UserMessage> {
        self.runs.take_failure().or_else(|| self.models.take_failure())
    }

    // MARK: Editor

    /// The editor's view of `subtitles`.
    pub fn editor_view(source: MaterialReference, subtitles: Subtitles) -> SubtitleEditorView {
        let cues = subtitles
            .cues
            .iter()
            .map(|cue| SubtitleCueView {
                id: cue.id.to_string(),
                time: text::time_range(cue),
                can_merge: subtitles.can_merge(cue.id),
                can_split: subtitles.can_split(cue.id),
            })
            .collect();
        SubtitleEditorView {
            material_id: source.material_id.to_string(),
            version_id: source.version_id.to_string(),
            title: text::TITLE.to_owned(),
            summary: text::summary(&subtitles),
            subtitles,
            cues,
        }
    }

    /// Opens the editor on the item's current version.
    pub async fn editor(&self, material_id: Uuid) -> Option<SubtitleEditorView> {
        let target = self.target(material_id).await?;
        Some(Self::editor_view(target.source, target.subtitles?))
    }

    /// Done: saves onto the version the editor opened on, found among the item's versions, only over what the editor
    /// opened with, unless `replace`.
    pub async fn save(
        &self,
        source: MaterialReference,
        opened: &Subtitles,
        edited: Subtitles,
        replace: bool,
    ) -> SubtitleSaveOutcome {
        let stored = match self.store.material(source.material_id).await {
            Ok(material) => material.and_then(|material| {
                material
                    .versions()
                    .into_iter()
                    .find(|version| version.id == source.version_id)
                    .map(|version| version.subtitles.clone())
            }),
            Err(error) => {
                return SubtitleSaveOutcome::Failed {
                    message: UserMessage::new(text::NOT_SAVED_TITLE, error.to_string(), None),
                };
            }
        };
        match SubtitleSave::decide(opened, stored.as_ref().map(Option::as_ref)) {
            SubtitleSave::VersionGone => {
                return SubtitleSaveOutcome::Failed {
                    message: UserMessage::new(text::NOT_SAVED_TITLE, text::VERSION_GONE_MESSAGE, None),
                };
            }
            SubtitleSave::ChangedMeanwhile if !replace => {
                return SubtitleSaveOutcome::ChangedMeanwhile {
                    title: text::CHANGED_MEANWHILE_TITLE.to_owned(),
                    message: text::CHANGED_MEANWHILE_MESSAGE.to_owned(),
                };
            }
            _ => {}
        }
        match self.store.set_subtitles(Some(edited), source).await {
            Ok(()) => SubtitleSaveOutcome::Saved,
            Err(error) => SubtitleSaveOutcome::Failed {
                message: UserMessage::new(text::NOT_SAVED_TITLE, error.to_string(), None),
            },
        }
    }

    /// Writes the cues as `format` into `path`, never over a file there.
    pub fn export(cues: &[SubtitleCue], format: SubtitleFormat, path: &Path) -> Result<(), String> {
        let mut file = std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(format.text(cues).as_bytes())
            .map_err(|error| error.to_string())
    }

    /// The export's suggested name: the video's name with the language and the format's extension.
    pub fn export_name(video: &Path, language: &str, format: SubtitleFormat) -> String {
        let stem = video
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{stem}.{language}.{}", format.file_extension())
    }

    // MARK: Settings

    pub fn settings(&self, is_available: bool) -> SubtitleSettingsView {
        let size = self.models.size();
        SubtitleSettingsView {
            engines: engine_options(),
            default_engine: self.runs.default_engine(),
            auto_note: text::AUTO_NOTE.to_owned(),
            models_folder: text::MODELS_FOLDER.to_owned(),
            model_name: WhisperModel::NAME.to_owned(),
            model_summary: text::WHISPER_SUMMARY.to_owned(),
            model_credit: text::WHISPER_CREDIT.to_owned(),
            model_facts: text::whisper_facts(),
            model_languages: text::whisper_languages(),
            model_url: WhisperModel::model_url(),
            license: WhisperModel::LICENSE.to_owned(),
            license_url: WhisperModel::LICENSE_URL.to_owned(),
            model_state: self.models.state(),
            model_size: (size > 0).then(|| crate::settings::format_bytes(size)),
            is_available,
            failure: self.models.failure(),
        }
    }

    pub fn set_default_engine(&self, engine: SubtitleEngine) {
        self.runs.set_default_engine(engine);
    }

    /// The service of the app: whisper.cpp with its model in the app data folder's "Speech Models", transcripts cached
    /// in the cache folder, subtitles saved on the version they were made from.
    pub fn open(
        layout: &jutsu_store::FileLayout,
        store: Arc<Store>,
        preferences: Arc<crate::preferences::PreferencesStore>,
        connection: Arc<crate::connection::ConnectionStore>,
    ) -> Arc<Self> {
        let root = Self::models_root(&layout.root);
        let downloader = jutsu_media::ModelDownloader::new(reqwest::Client::new());
        let speech = jutsu_media::OnDeviceSpeech::new(
            jutsu_media::FfmpegTools::from_environment(),
            SpeechModels::folder_in(&root),
            downloader.clone(),
            Some(layout.cache_folder().join("Transcripts")),
        );
        let saving = Arc::clone(&store);
        let register: super::runs::Register =
            Arc::new(move |request: SubtitleRequest, subtitles: Subtitles| {
                let store = Arc::clone(&saving);
                Box::pin(async move {
                    store
                        .set_subtitles(Some(subtitles), request.source)
                        .await
                        .map_err(|error| error.to_string())
                })
            });
        let runs = SubtitleRuns::new(Arc::new(speech), preferences, register);
        let models = SpeechModels::new(&root, Arc::new(downloader));
        let agent = AgentSubtitleRuns::new(
            Arc::clone(&runs),
            Some(Arc::clone(&models)),
            Arc::new(move || {
                let state = connection.state();
                crate::agents::budget::unit_for(state.kind().unwrap_or(jutsu_core::ProviderKind::ApiKey))
            }),
        );
        Self::new(store, runs, models, agent)
    }

    /// Whether this build hears speech at all.
    pub fn is_available() -> bool {
        jutsu_media::OnDeviceSpeech::HAS_RECOGNIZER && jutsu_media::FfmpegTools::from_environment().is_some()
    }

    /// The models' folder under the app's data folder.
    pub fn models_root(root: &Path) -> PathBuf {
        root.join(SpeechModels::FOLDER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_never_writes_over_a_file() {
        let folder = tempfile::tempdir().expect("a temp dir");
        let path = folder.path().join("talk.en-US.srt");
        let cues = [SubtitleCue::new(0.2, 1.5, "Hello")];
        SubtitleService::export(&cues, SubtitleFormat::Srt, &path).expect("written");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "1\n00:00:00,200 --> 00:00:01,500\nHello\n"
        );
        assert!(SubtitleService::export(&cues, SubtitleFormat::Vtt, &path).is_err());
        assert_eq!(
            SubtitleService::export_name(Path::new("/tmp/Talk take.mp4"), "en-US", SubtitleFormat::Vtt),
            "Talk take.en-US.vtt"
        );
    }
}
