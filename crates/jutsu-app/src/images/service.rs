//! The Image side of Create as the window sees it (`CreateImagePanel.swift`, `ImageGenerateController`,
//! `AppEnvironment+References.swift`): the form, its price, Generate, the stage of results, the image draft, the
//! connection's image models and saving each image to the Library.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jutsu_core::{
    Cost, ImageModel, ImageModelInput, ImageProvider, LocalImage, MediaPreparing, Mode, ProviderKind,
    TemplateKind,
};
use jutsu_store::{FileLayout, Store};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

use super::characters::{CharacterBuilder, CharacterSheetLog};
use super::form::{IMAGE_TAGLINE, UNKNOWN_PRICE_NOTICE, image_price, kind_title, price_text};
use super::generator::{
    GeneratedImage, ImageRun, PaidStart, ReferenceGenerator, RunState, SpendStatusReader, UploadFn,
};
use super::job_log::ImageJobLog;
use super::saving::ImageSaver;
use super::spend_log::ImageSpendLog;
use crate::agents::image_queue::AgentImageQueue;
use crate::connection::{ConnectionStore, UserMessage};
use crate::create::Notice;
use crate::create::draft::CreateDraftStore;
use crate::create::service::SAVE_DELAY;
use crate::preferences::{Preferences, PreferencesStore};
use crate::queue::QueueConfirmation;

/// Price quotes wait this long after the last change, so typing doesn't ask on every key.
const QUOTE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(400);

/// The side Create shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CreateSide {
    Video,
    Image,
}

impl From<TemplateKind> for CreateSide {
    fn from(kind: TemplateKind) -> Self {
        match kind {
            TemplateKind::Video => Self::Video,
            TemplateKind::Image => Self::Image,
        }
    }
}

impl From<CreateSide> for TemplateKind {
    fn from(side: CreateSide) -> Self {
        match side {
            CreateSide::Video => Self::Video,
            CreateSide::Image => Self::Image,
        }
    }
}

/// What the window can change on the image form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImageEdit {
    SetPrompt {
        text: String,
    },
    SelectModel {
        id: String,
    },
    SetAspectRatio {
        ratio: String,
    },
    SetCount {
        count: i32,
    },
    /// The image to edit; `None` removes it.
    SetInput {
        path: Option<String>,
    },
    /// Files dropped on the Image to edit row: the first image becomes the image to edit; none changes nothing.
    DropInput {
        paths: Vec<String>,
    },
}

/// One model of the picker: "Soul 2 · $0.0032 an image".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageModelChoice {
    pub id: String,
    pub title: String,
}

/// The image to edit row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageInputView {
    /// "Required" or "Optional".
    pub detail: String,
    pub path: Option<String>,
    pub name: Option<String>,
}

/// Generate with its price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerateView {
    pub is_generating: bool,
    pub enabled: bool,
    /// "$0.0128"; `None` while it is being quoted, can't be, or is only known after the run.
    pub price: Option<String>,
    /// "$0.0128 · Price from Higgsfield", for the hover hint.
    pub hint: String,
}

/// What Generate's button can't say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageNoticeView {
    pub text: String,
    /// A price that couldn't be quoted: smaller and fainter.
    pub is_quiet: bool,
}

/// How a request row reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum RequestStatus {
    Working,
    Finished,
    Failed,
    Canceled,
    /// Stopped before anything was sent, until the person decides (an agent's request over its limits).
    Waiting,
}

/// One request on a line (`RequestRow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageRequestRow {
    pub id: String,
    /// "Soul 2 · 4 images".
    pub title: String,
    pub status: RequestStatus,
    /// "Making the images…", the failure, "Canceled", or why an agent's request waits.
    pub detail: Option<String>,
    pub can_cancel: bool,
    /// Under the title: who asked for it, for an agent's request ("Agent · claude-code · Allowed up to $2.00").
    pub subtitle: Option<String>,
    /// An agent's request the spending guard held: Review… opens the person's decision, which says this.
    pub review: Option<String>,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageTile {
    pub id: String,
    /// The saved copy; `None` until saved.
    pub path: Option<String>,
    /// Where the provider serves it, shown until the copy is saved.
    pub url: String,
    pub prompt: String,
    /// The model that made it, beside "Image 2 of 4" in the large preview.
    pub model: String,
    /// Added to Create's references: the strip offers no Move to Trash for it.
    pub is_picked: bool,
}

/// References › Generate…'s picks (`GenerateReferenceSheet`): the results come from the image snapshot, the picks and
/// what they say from here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ReferencePicksView {
    /// Generate… opens: an image connection is set.
    pub offered: bool,
    /// The picked images, in the order they were picked.
    pub picked: Vec<String>,
    /// The images already added to the references: they show Added and can't be picked again.
    pub added: Vec<String>,
    /// One more image can be picked: Create has room for it.
    pub can_pick: bool,
    /// "Pick up to 3", "2 of 3 picked"; `None` without results.
    pub detail: Option<String>,
}

/// Prices and requests that couldn't be written down (`UnsavedRecordsWarning`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UnsavedRecordsView {
    pub reason: String,
    pub detail: String,
    pub job_ids: Vec<String>,
}

/// The Image side of Create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageSnapshot {
    pub side: CreateSide,
    pub tagline: String,
    pub prompt: String,
    /// Shown instead of the form: no models, with why.
    pub unavailable_reason: Option<String>,
    /// The option rows' places while the first list of models loads.
    pub loading_models: bool,
    pub models: Vec<ImageModelChoice>,
    pub model_id: Option<String>,
    pub aspect_ratios: Vec<String>,
    pub aspect_ratio: Option<String>,
    /// Shown when the model makes more than one count.
    pub variant_counts: Vec<i32>,
    pub count: i32,
    /// `None` for a model that edits no image.
    pub input: Option<ImageInputView>,
    pub generate: ImageGenerateView,
    pub notice: Option<ImageNoticeView>,
    /// The requests that didn't finish: running, failed or canceled.
    pub runs: Vec<ImageRequestRow>,
    /// Every request, for the Queue's Images section.
    pub requests: Vec<ImageRequestRow>,
    /// The results, newest first.
    pub images: Vec<ImageTile>,
    /// The result shown large: the picked one, or the newest.
    pub current: Option<ImageTile>,
    /// Places for the images running requests will make.
    pub pending_tiles: i32,
    /// The request shown as a spinner while it has no image yet.
    pub running: Option<ImageRequestRow>,
    pub unsaved: Option<UnsavedRecordsView>,
    /// A save that failed, for an alert.
    pub save_failure: Option<UserMessage>,
}

/// What Generate did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImageGenerateOutcome {
    Started,
    /// Over today's cap, or the price isn't known under one: ask, then generate again approved.
    Confirm {
        confirmation: QueueConfirmation,
    },
    /// Why it can't run.
    Blocked {
        reason: String,
    },
    Failed {
        notice: Notice,
    },
    /// Nothing was sent: another connection came first.
    NotStarted,
}

/// What the service reaches the rest of the app through.
#[derive(Clone)]
pub struct ImageDeps {
    pub layout: FileLayout,
    pub store: Arc<Store>,
    pub preferences: Arc<PreferencesStore>,
    pub connection: Arc<ConnectionStore>,
    /// Prepares the image to edit like a reference of a job.
    pub media: Arc<dyn MediaPreparing>,
    pub spend: Arc<ImageSpendLog>,
    pub utc_offset: UtcOffset,
}

struct ServiceState {
    side: CreateSide,
    provider: Option<Arc<dyn ImageProvider>>,
    /// Goes up with each provider, so a list of models read for an older one isn't used.
    generation: u64,
    /// The result picked to show large; `None` follows the newest.
    shown: Option<Uuid>,
    is_starting: bool,
    /// The price key last quoted, with the provider generation.
    priced: Option<(u64, String)>,
    edits: u64,
    /// Create Character.
    characters: character_sheet::CharacterState,
}

/// The Image side of Create.
pub struct ImageService {
    deps: ImageDeps,
    generator: Arc<ReferenceGenerator>,
    saver: Arc<ImageSaver>,
    characters: Arc<CharacterBuilder>,
    drafts: CreateDraftStore,
    state: Mutex<ServiceState>,
    changes: watch::Sender<u64>,
    /// The agents' `create_image` requests, once `attach_agent_images` set them up.
    agent_images: std::sync::OnceLock<Arc<AgentImageQueue>>,
}

impl std::fmt::Debug for ImageService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ImageService").finish_non_exhaustive()
    }
}

/// Today's spend (the jobs and the image charges) and the cap of a connection kind.
pub fn spend_status_reader(
    store: &Arc<Store>,
    preferences: &Arc<PreferencesStore>,
    spend: &Arc<ImageSpendLog>,
    offset: UtcOffset,
) -> SpendStatusReader {
    let (store, preferences, spend) = (store.clone(), preferences.clone(), spend.clone());
    Arc::new(move |kind: ProviderKind| {
        let (store, preferences, spend) = (store.clone(), preferences.clone(), spend.clone());
        Box::pin(async move {
            let Some(cap) = preferences.get().spend_cap(Some(kind)) else {
                return Ok(None);
            };
            let committed = crate::settings::spending::committed_today_excluding(
                &store,
                &spend,
                cap.unit,
                OffsetDateTime::now_utc(),
                offset,
                None,
            )
            .await?;
            Ok(Some(jutsu_core::SpendStatus {
                committed_today: committed,
                cap: Some(cap),
            }))
        })
    })
}

impl ImageService {
    /// Opens with the saved image draft and side.
    pub fn new(deps: ImageDeps) -> Arc<Self> {
        let job_log = Arc::new(ImageJobLog::open(Some(&deps.layout.root)));
        let reader = spend_status_reader(&deps.store, &deps.preferences, &deps.spend, deps.utc_offset);
        let generator = ReferenceGenerator::new(
            deps.spend.clone(),
            job_log,
            reader.clone(),
            ReferenceGenerator::real_sleep(),
        );
        let characters = CharacterBuilder::new(
            Arc::new(CharacterSheetLog::open(Some(&deps.layout.root))),
            deps.spend.clone(),
            reader.clone(),
            ReferenceGenerator::real_sleep(),
        );
        let saver = ImageSaver::new(
            deps.store.clone(),
            deps.layout.clone(),
            deps.preferences.clone(),
            deps.utc_offset,
        );
        let drafts = CreateDraftStore::new(deps.layout.create_draft_path());
        if let Some(draft) = drafts.restore_image() {
            generator.edit_form(|form| form.restore(&draft));
        }
        let side = drafts.load_kind().into();
        let service = Arc::new(Self {
            deps,
            generator,
            saver,
            characters,
            drafts,
            state: Mutex::new(ServiceState {
                side,
                provider: None,
                generation: 0,
                shown: None,
                is_starting: false,
                priced: None,
                edits: 0,
                characters: character_sheet::CharacterState::default(),
            }),
            changes: watch::Sender::new(0),
            agent_images: std::sync::OnceLock::new(),
        });
        service.wire();
        service.wire_characters();
        service
    }

    /// Saves each image as it arrives, and follows the generator's changes.
    fn wire(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.generator.set_on_images(Arc::new(move |images| {
            let Some(service) = weak.upgrade() else {
                return;
            };
            let saved = Arc::downgrade(&service);
            service.saver.save(
                images,
                std::collections::HashMap::new(),
                Arc::new(move |image: &GeneratedImage, file| {
                    let Some(service) = saved.upgrade() else {
                        return;
                    };
                    if service.generator.contains(image.id) {
                        service.generator.set_file(image.id, file);
                    } else {
                        // Deleted from the results while it was downloading.
                        let trash = service.saver.clone();
                        let id = image.id;
                        tokio::spawn(async move {
                            let _ = trash.trash(id).await;
                        });
                    }
                }),
            );
        }));
        let connection = Arc::downgrade(&self.deps.connection);
        self.generator.set_on_sign_in_expired(Arc::new(move || {
            if let Some(connection) = connection.upgrade() {
                connection.mark_needs_attention();
            }
        }));
    }

    fn locked(&self) -> MutexGuard<'_, ServiceState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The image charges toward today's spend, which `capabilities` reads with the jobs'.
    pub fn spend_log(&self) -> &Arc<ImageSpendLog> {
        &self.deps.spend
    }

    pub fn generator(&self) -> &Arc<ReferenceGenerator> {
        &self.generator
    }

    /// Bumps whenever the snapshot changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn notify(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    /// Follows the generator's changes, and saves the images a quit left unsaved. Run once, at launch.
    pub fn start(self: &Arc<Self>) {
        let mut changes = self.generator.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.notify();
            }
        });
        let saver = self.saver.clone();
        tokio::spawn(async move { saver.resume_pending(OffsetDateTime::now_utc()).await });
    }

    /// Waits for the submits already sent and the images being saved; writes the draft.
    pub async fn stop(&self) {
        self.generator.finish_submits().await;
        self.characters.finish_submit().await;
        self.saver.finish_saving().await;
        self.save_now().await;
    }

    // MARK: Connection

    /// A new connection, or none: the old one's requests stop being followed, the new one's are followed again,
    /// and its image models are read.
    pub fn set_provider(self: &Arc<Self>, provider: Option<Arc<dyn ImageProvider>>) {
        self.generator.reset();
        let generation = {
            let mut state = self.locked();
            state.generation += 1;
            state.provider.clone_from(&provider);
            state.priced = None;
            state.generation
        };
        self.notify();
        let Some(provider) = provider else {
            self.generator.edit_form(|form| form.use_models(Vec::new(), None));
            return;
        };
        self.generator.resume(&provider);
        if let Some(queue) = self.agent_images.get() {
            queue.resume(&provider, &self.uploader(&provider));
        }
        let key = Preferences::image_models_key(provider.kind(), provider.owner().as_deref());
        let saved: Vec<ImageModel> = self
            .deps
            .preferences
            .value(&key)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        self.generator.edit_form(|form| form.use_saved_models(saved));
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            if let Some(service) = weak.upgrade() {
                service.read_models(provider, generation).await;
            }
        });
    }

    /// Reads the connection's image models and keeps them for the next launch.
    async fn read_models(&self, provider: Arc<dyn ImageProvider>, generation: u64) {
        let key = Preferences::image_models_key(provider.kind(), provider.owner().as_deref());
        let models = provider.image_models().await;
        // Read for a connection replaced meanwhile: not this one's.
        if self.locked().generation != generation {
            return;
        }
        match models {
            Ok(models) => {
                if let Ok(value) = serde_json::to_value(&models)
                    && let Err(error) = self.deps.preferences.set_value(&key, Some(value))
                {
                    tracing::warn!(%error, "can't save the image models");
                }
                self.generator.edit_form(|form| form.use_models(models, None));
            }
            Err(error) => {
                let reason =
                    UserMessage::from_error(&error, Some(provider.kind())).map(|message| message.message);
                self.generator
                    .edit_form(|form| form.use_models(Vec::new(), reason));
            }
        }
    }

    /// Reads the connection's image models now while its own list isn't in (`ReferenceGenerator.needsModelRead`).
    pub async fn read_models_if_needed(&self) {
        if !self.generator.form().needs_model_read() {
            return;
        }
        let (provider, generation) = {
            let state = self.locked();
            (state.provider.clone(), state.generation)
        };
        if let Some(provider) = provider {
            self.read_models(provider, generation).await;
        }
    }

    // MARK: Agents

    /// Sets up the agents' `create_image` requests, sent through this side's runs and the agent spend `ledger`. Run
    /// once, before the first connection.
    pub fn attach_agent_images(&self, ledger: Arc<dyn jutsu_core::AgentSpendLedger>) {
        let budgets = crate::agents::preferences::AgentPreferences::new(self.deps.preferences.clone());
        let queue = AgentImageQueue::new(
            Some(&self.deps.layout.root),
            self.generator.clone(),
            ledger,
            Arc::new(move |kind| budgets.daily_budget(kind)),
            Arc::new(OffsetDateTime::now_utc),
        );
        if self.agent_images.set(queue).is_err() {
            tracing::warn!("the agent image requests were set up twice");
        }
    }

    pub fn agent_images(&self) -> Option<&Arc<AgentImageQueue>> {
        self.agent_images.get()
    }

    /// At launch, before the first connection: reconciles the agent image requests with the ledger and shows the
    /// held ones again.
    pub async fn restore_agent_images(&self) {
        if let Some(queue) = self.agent_images.get() {
            queue.restore().await;
        }
    }

    /// The connection images are made on, with how its inputs are uploaded; `None` without one.
    pub fn agent_provider(&self) -> Option<(Arc<dyn ImageProvider>, UploadFn)> {
        let provider = self.provider()?;
        let upload = self.uploader(&provider);
        Some((provider, upload))
    }

    /// The person's Send on an agent's image request the spending guard held: it goes out on the connection that
    /// took it, at the price they saw.
    pub fn send_held(&self, id: Uuid) {
        let (Some(queue), Some((provider, upload))) = (self.agent_images.get(), self.agent_provider()) else {
            return;
        };
        queue.approve(id, provider, upload);
    }

    fn provider(&self) -> Option<Arc<dyn ImageProvider>> {
        self.locked().provider.clone()
    }

    /// Prepares the image to edit like a reference of a job, then uploads it.
    fn uploader(&self, provider: &Arc<dyn ImageProvider>) -> UploadFn {
        let (media, provider) = (self.deps.media.clone(), provider.clone());
        Arc::new(move |image: LocalImage| {
            let (media, provider) = (media.clone(), provider.clone());
            Box::pin(async move { provider.upload(media.prepare_image(&image).await?).await })
        })
    }

    // MARK: Snapshot

    /// What the screen shows now; asks for the price when the request changed.
    #[allow(clippy::too_many_lines, reason = "one field per part of the screen")]
    pub fn snapshot(self: &Arc<Self>) -> ImageSnapshot {
        self.schedule_price();
        let form = self.generator.form();
        let (side, shown, is_starting) = {
            let state = self.locked();
            (state.side, state.shown, state.is_starting)
        };
        let model = form.model();
        let price = self.generator.price();
        let runs = self.generator.runs();
        let images = self.generator.images();
        let tiles: Vec<ImageTile> = images.iter().map(tile).collect();
        let current = shown
            .and_then(|id| images.iter().find(|image| image.id == id))
            .or_else(|| images.first())
            .map(tile);
        let rows: Vec<ImageRequestRow> = runs.iter().map(request_row).collect();
        let unfinished: Vec<ImageRequestRow> = runs
            .iter()
            .filter(|run| run.state != RunState::Finished)
            .map(request_row)
            .collect();
        let pending_tiles: i64 = runs
            .iter()
            .filter(|run| run.state == RunState::Running)
            .map(|run| run.count)
            .sum();
        let running = runs
            .iter()
            .find(|run| run.state == RunState::Running)
            .map(request_row);
        let is_generating = is_starting || self.generator.is_running();
        let blocker = form.blocker();
        let notice = if let (Some(blocker), Some(_)) = (&blocker, model) {
            Some(ImageNoticeView {
                text: blocker.clone(),
                is_quiet: false,
            })
        } else if price.as_ref().is_some_and(Cost::is_unknown_price) {
            Some(ImageNoticeView {
                text: UNKNOWN_PRICE_NOTICE.to_owned(),
                is_quiet: false,
            })
        } else {
            self.generator
                .price_failure()
                .map(|text| ImageNoticeView { text, is_quiet: true })
        };
        ImageSnapshot {
            side,
            tagline: IMAGE_TAGLINE.to_owned(),
            prompt: form.prompt.clone(),
            unavailable_reason: form
                .unavailable_reason()
                .filter(|_| form.models().is_empty())
                .map(str::to_owned),
            loading_models: model.is_none(),
            models: form.models().iter().map(model_choice).collect(),
            model_id: form.model_id().map(str::to_owned),
            aspect_ratios: model.map(|model| model.aspect_ratios.clone()).unwrap_or_default(),
            aspect_ratio: form.aspect_ratio(None, Mode::MotionTransfer),
            variant_counts: model
                .filter(|model| model.variant_counts.len() > 1)
                .map(|model| {
                    model
                        .variant_counts
                        .iter()
                        .map(|count| ipc_count(*count))
                        .collect()
                })
                .unwrap_or_default(),
            count: ipc_count(form.count),
            input: model
                .filter(|model| model.input != ImageModelInput::None)
                .map(|model| ImageInputView {
                    detail: if model.input == ImageModelInput::Required {
                        "Required"
                    } else {
                        "Optional"
                    }
                    .to_owned(),
                    path: form
                        .input_image
                        .as_ref()
                        .and_then(|image| local_path(&image.file_url)),
                    name: form.input_image.as_ref().map(|image| file_name(&image.file_url)),
                }),
            generate: ImageGenerateView {
                is_generating,
                enabled: blocker.is_none() && !is_generating,
                price: price
                    .as_ref()
                    .filter(|price| !price.is_unknown_price())
                    .map(image_price),
                hint: price.as_ref().map(price_text).unwrap_or_default(),
            },
            notice,
            runs: unfinished,
            requests: rows,
            images: tiles,
            current,
            pending_tiles: i32::try_from(pending_tiles).unwrap_or(i32::MAX),
            running,
            unsaved: self.unsaved(),
            save_failure: self.saver.failure(),
        }
    }

    /// Reads the price again, a moment after the request last changed.
    fn schedule_price(self: &Arc<Self>) {
        let Some(provider) = self.provider() else {
            return;
        };
        let key = self.generator.form().price_key(None, Mode::MotionTransfer);
        let edits = {
            let mut state = self.locked();
            let priced = (state.generation, key);
            if state.priced.as_ref() == Some(&priced) {
                return;
            }
            state.priced = Some(priced);
            state.edits
        };
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(QUOTE_DEBOUNCE).await;
            let Some(service) = weak.upgrade() else {
                return;
            };
            if service.locked().edits != edits {
                return;
            }
            let upload = service.uploader(&provider);
            service
                .generator
                .refresh_price(&provider, None, Mode::MotionTransfer, &upload)
                .await;
        });
    }

    fn unsaved(&self) -> Option<UnsavedRecordsView> {
        let job_log = self.generator.job_log();
        let sheets = self.characters.log();
        let error = self
            .deps
            .spend
            .save_error()
            .or_else(|| job_log.save_error())
            .or_else(|| sheets.save_error())?;
        let mut job_ids = job_log.unsaved_ids();
        job_ids.extend(sheets.unsaved_ids());
        let detail = if job_ids.is_empty() {
            "Prices and requests from now on are kept only until you quit.".to_owned()
        } else {
            format!(
                "Higgsfield accepted {} jobs that would be lost on quit. Copy their IDs to find them in your Higgsfield history.",
                job_ids.len()
            )
        };
        Some(UnsavedRecordsView {
            reason: error.to_string(),
            detail,
            job_ids,
        })
    }

    // MARK: Edits

    fn edited(self: &Arc<Self>) {
        let edits = {
            let mut state = self.locked();
            state.edits += 1;
            state.edits
        };
        self.notify();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            if let Some(service) = weak.upgrade()
                && service.locked().edits == edits
            {
                service.save_now().await;
            }
        });
    }

    /// Writes the image draft and the side now.
    pub async fn save_now(&self) {
        let draft = self.generator.form().snapshot();
        let side = self.locked().side;
        let path = self.drafts.path().to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            CreateDraftStore::new(path).save_image(draft.has_inputs().then_some(&draft), side.into())
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "can't save the image draft"),
            Err(error) => tracing::error!(%error, "can't save the image draft"),
        }
    }

    pub fn edit(self: &Arc<Self>, edit: ImageEdit) -> ImageSnapshot {
        self.generator.edit_form(|form| match edit {
            ImageEdit::SetPrompt { text } => form.prompt = text,
            ImageEdit::SelectModel { id } => form.select_model(&id),
            ImageEdit::SetAspectRatio { ratio } => form.chosen_aspect_ratio = Some(ratio),
            ImageEdit::SetCount { count } => {
                let count = i64::from(count);
                if form
                    .model()
                    .is_none_or(|model| model.variant_counts.contains(&count))
                {
                    form.count = count;
                }
            }
            ImageEdit::SetInput { path } => {
                form.input_image = path
                    .and_then(|path| Url::from_file_path(Path::new(&path)).ok())
                    .map(LocalImage::new);
            }
            ImageEdit::DropInput { paths } => {
                if let Some(image) = paths
                    .iter()
                    .map(Path::new)
                    .find(|path| jutsu_media::frames::is_image(path))
                    .and_then(|path| Url::from_file_path(path).ok())
                {
                    form.input_image = Some(LocalImage::new(image));
                }
            }
        });
        self.edited();
        self.snapshot()
    }

    /// The side Create shows; kept in the draft.
    pub fn set_side(self: &Arc<Self>, side: CreateSide) {
        self.locked().side = side;
        self.edited();
    }

    pub fn side(&self) -> CreateSide {
        self.locked().side
    }

    /// The form as an image template.
    pub fn make_template(&self, name: &str) -> jutsu_core::JobTemplate {
        self.generator
            .form()
            .make_image_template(name, Uuid::new_v4(), OffsetDateTime::now_utc())
    }

    /// Fills the form from an image template and shows the Image side.
    pub fn apply_template(self: &Arc<Self>, template: &jutsu_core::JobTemplate) -> Option<Notice> {
        let notice = self
            .generator
            .edit_form(|form| form.apply_image_template(template));
        self.locked().side = CreateSide::Image;
        self.edited();
        notice
    }

    /// "Image · a knight": what Save as Template suggests.
    pub fn suggested_template_name(&self) -> String {
        crate::templates::suggested_name(kind_title(TemplateKind::Image), &self.generator.form().prompt)
    }

    // MARK: Generate

    /// Asks first when the price takes today past the cap, or isn't known under one; call again `approved`.
    pub async fn generate(self: &Arc<Self>, approved: bool) -> ImageGenerateOutcome {
        let form = self.generator.form();
        if let Some(reason) = form.blocker() {
            return ImageGenerateOutcome::Blocked { reason };
        }
        let Some(provider) = self.provider() else {
            return ImageGenerateOutcome::Failed {
                notice: Notice {
                    title: "Not connected".to_owned(),
                    detail: "Connect Higgsfield again to generate images.".to_owned(),
                },
            };
        };
        {
            let mut state = self.locked();
            if state.is_starting || self.generator.is_running() {
                return ImageGenerateOutcome::NotStarted;
            }
            state.is_starting = true;
        }
        self.notify();
        let price = self.generator.price().or_else(|| form.list_price());
        let upload = self.uploader(&provider);
        let started = self
            .generator
            .generate(provider, None, Mode::MotionTransfer, price, approved, upload)
            .await;
        self.locked().is_starting = false;
        self.notify();
        match started {
            PaidStart::Started => ImageGenerateOutcome::Started,
            PaidStart::NeedsConfirmation(confirmation) => ImageGenerateOutcome::Confirm { confirmation },
            PaidStart::NotStarted => ImageGenerateOutcome::NotStarted,
        }
    }

    /// An agent's request goes through its queue, which records how it ended; the person's own straight to Generate.
    pub fn cancel(&self, run: Uuid) {
        let is_agents = self
            .generator
            .runs()
            .iter()
            .any(|shown| shown.id == run && shown.agent.is_some());
        match self.agent_images.get() {
            Some(queue) if is_agents => {
                queue.cancel(run);
            }
            _ => self.generator.cancel(run),
        }
    }

    /// Shows the result large; `None` follows the newest.
    pub fn show(&self, image: Option<Uuid>) {
        self.locked().shown = image;
        self.notify();
    }

    /// Takes a result off the list and moves its saved copy to the Trash, also out of Library › Images. Higgsfield
    /// keeps its copy.
    pub async fn delete(&self, image: Uuid) -> Result<(), Notice> {
        {
            let mut state = self.locked();
            if state.shown == Some(image) {
                state.shown = None;
            }
        }
        if self.generator.remove(image).is_none() {
            return Ok(());
        }
        self.saver.trash(image).await.map_err(|message| Notice {
            title: "Couldn't move the image to the Trash".to_owned(),
            detail: message,
        })
    }

    /// Drops the results and the requests that are over.
    pub fn clear(&self) {
        self.locked().shown = None;
        self.generator.clear_results();
    }

    // MARK: Generate… for References

    /// The picks of Generate…, with `room` more references Create takes.
    pub fn reference_picks(&self, room: usize) -> ReferencePicksView {
        let selection = self.generator.selection();
        let images = self.generator.images();
        let picked = selection.len();
        ReferencePicksView {
            offered: self.locked().provider.is_some(),
            picked: selection.iter().map(Uuid::to_string).collect(),
            added: images
                .iter()
                .filter(|image| image.is_picked)
                .map(|image| image.id.to_string())
                .collect(),
            can_pick: picked < room,
            detail: (!images.is_empty()).then(|| {
                if picked == 0 {
                    format!("Pick up to {room}")
                } else {
                    format!("{picked} of {room} picked")
                }
            }),
        }
    }

    /// A click on a result: picks it, or unpicks it.
    pub fn toggle_reference_pick(&self, id: Uuid, room: usize) -> ReferencePicksView {
        self.generator.toggle_selection(id, room);
        self.reference_picks(room)
    }

    /// Generate… closed: its picks go, the images stay for the Image side of Create.
    pub fn clear_reference_picks(&self) {
        self.generator.clear_selection();
    }

    /// The picks to add, in the order they were picked.
    pub fn picked_images(&self) -> Vec<GeneratedImage> {
        self.generator.selected_images()
    }

    /// `id` was added to Create's references.
    pub fn mark_added(&self, id: Uuid) {
        self.generator.mark_picked(id);
    }

    /// Writes the prices and requests again after a failed write.
    pub async fn retry_saving_records(&self) {
        let (spend, jobs, sheets) = (
            self.deps.spend.clone(),
            self.generator.job_log().clone(),
            self.characters.log().clone(),
        );
        let _ = tokio::task::spawn_blocking(move || {
            spend.retry_save();
            jobs.retry_save();
            sheets.retry_save();
        })
        .await;
        self.notify();
    }

    /// The save failure was shown.
    pub fn dismiss_save_failure(&self) {
        let _ = self.saver.take_failure();
        self.notify();
    }
}

fn ipc_count(count: i64) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// "Soul 2 · $0.0032 an image".
fn model_choice(model: &ImageModel) -> ImageModelChoice {
    let title = match &model.list_price {
        Some(price) => format!("{} · {} an image", model.title, image_price(price)),
        None => model.title.clone(),
    };
    ImageModelChoice {
        id: model.id.clone(),
        title,
    }
}

fn request_row(run: &ImageRun) -> ImageRequestRow {
    let (status, detail) = match &run.state {
        RunState::Running => (RequestStatus::Working, Some("Making the images…".to_owned())),
        RunState::Finished => (RequestStatus::Finished, None),
        RunState::Failed(message) | RunState::Interrupted(message) => {
            (RequestStatus::Failed, Some(message.clone()))
        }
        RunState::Canceled => (RequestStatus::Canceled, Some("Canceled".to_owned())),
        RunState::Held(message) => (RequestStatus::Waiting, Some(message.clone())),
    };
    let agent_title = run.agent.as_ref().map(crate::queue::row::origin_text);
    let agent_detail = run
        .agent
        .as_ref()
        .map(|agent| format!("Allowed up to {}", crate::queue::row::money_text(agent.max_cost)));
    let subtitle = agent_title.clone().map(|title| {
        agent_detail
            .as_ref()
            .map_or(title.clone(), |detail| format!("{title} · {detail}"))
    });
    // What the person agrees to: who asked, what, at which price, and why it waited.
    let review = (status == RequestStatus::Waiting).then(|| {
        let price = run
            .price
            .as_ref()
            .map_or_else(|| UNKNOWN_PRICE_NOTICE.to_owned(), price_text);
        [
            agent_title.unwrap_or_default(),
            run.title(),
            price,
            agent_detail.unwrap_or_default(),
            detail.clone().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
    });
    ImageRequestRow {
        id: run.id.to_string(),
        title: run.title(),
        can_cancel: matches!(status, RequestStatus::Working | RequestStatus::Waiting),
        status,
        detail,
        subtitle,
        review,
    }
}

fn tile(image: &GeneratedImage) -> ImageTile {
    ImageTile {
        id: image.id.to_string(),
        path: image
            .file
            .as_ref()
            .map(|file| file.to_string_lossy().into_owned()),
        url: image.url.to_string(),
        prompt: image.prompt.clone(),
        model: image.model.clone(),
        is_picked: image.is_picked,
    }
}

fn local_path(url: &Url) -> Option<String> {
    url.to_file_path()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

fn file_name(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(|name| {
            percent_encoding::percent_decode_str(name)
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_default()
}

#[path = "character_service.rs"]
pub mod character_sheet;

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
