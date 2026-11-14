//! The Create screen's live state (`CreateView`, `SubmitController`, `AppEnvironment+Create`): the video draft, its
//! media, the provider's quotes, the draft file, and the submit. Commands change it; `subscribe` says when to ask
//! for the snapshot again.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jutsu_core::{
    Capabilities, Cost, Job, JobSpec, KlingQuality, LocalImage, Pricing, Resolution, SpendCap, VideoControl,
    VideoModel,
};
use jutsu_engine::{EnqueueOptions, JobEngine};
use jutsu_media::MediaInfo;
use jutsu_media::hash::sha256_hex;
use jutsu_store::{FileLayout, Store};
use rust_decimal::Decimal;
use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

use super::CreateError;
use super::context::{CreateContext, EstimateCache, SubmitDecision};
use super::draft::{CreateDraftStore, replacement};
use super::edit::CreateEdit;
use super::form::{CreateForm, FrameSlot, Notice, SourceVideo};
use super::media::SourceMedia;
use super::view::{CreateSnapshot, MediaState, PlaybackView, snapshot};
use crate::connection::{ConnectionStore, UserMessage};
use crate::preferences::PreferencesStore;
use crate::provider::ProviderHandle;
use crate::queue::QueueConfirmation;
use crate::queue::service::{queued_notice, user_message};

mod gallery;
mod higgsfield;
mod links;
mod make_video;
mod speech;

/// Quotes wait this long after the last change, so dragging a handle doesn't ask on every step.
pub const QUOTE_DEBOUNCE: Duration = Duration::from_millis(400);
/// The draft file is written this long after the last change.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);
/// Frames on the trim strip.
const THUMBNAIL_COUNT: usize = 12;
/// Videos Open and a drop take as the source.
pub const VIDEO_EXTENSIONS: [&str; 9] = ["mp4", "mov", "m4v", "mkv", "webm", "avi", "hevc", "mts", "mxf"];
/// Images the References and frame pickers offer; a drop takes any image `jutsu-media` reads.
pub const IMAGE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "heic", "heif", "webp", "tiff"];
/// Recordings Speech takes: the audio types macOS knows by extension (`UTType` conforming to `.audio`), each read
/// through ffmpeg; MP3, WAV and M4A are sent as they are. Video containers (`webm`, `mp4`) are videos there too.
pub const AUDIO_EXTENSIONS: [&str; 23] = [
    "mp3", "wav", "m4a", "aac", "aif", "aiff", "caf", "flac", "ogg", "opus", "m4b", "m4r", "mp2", "mpga",
    "au", "snd", "aifc", "weba", "wma", "amr", "ac3", "oga", "adts",
];

/// What the service reaches the rest of the app through.
#[derive(Clone)]
pub struct CreateDeps {
    pub layout: FileLayout,
    pub store: Arc<Store>,
    pub preferences: Arc<PreferencesStore>,
    pub connection: Arc<ConnectionStore>,
    pub provider: ProviderHandle,
    pub engine: JobEngine,
    pub media: Arc<dyn SourceMedia>,
    pub utc_offset: UtcOffset,
    /// The image requests' prices, which count toward the daily cap beside the jobs.
    pub image_spend: Arc<crate::images::ImageSpendLog>,
    /// Prepares a character set's images for "Save to Higgsfield…" like a job's references.
    pub preparer: Arc<dyn jutsu_core::MediaPreparing>,
}

/// What Draft, Generate or Render final did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SubmitOutcome {
    /// "Final queued · ~$0.68", for the toast.
    Queued { notice: String },
    /// Over today's cap, or the price isn't known under one: ask, then submit again approved.
    Confirm { confirmation: QueueConfirmation },
    /// The button's blocker.
    Blocked { reason: String },
    /// Another submit is still being queued.
    Busy,
    /// The reason is the connection alert.
    Failed,
}

/// What loading a job's inputs did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LoadOutcome {
    /// The draft it replaced lost inputs: offer Undo ("Draft replaced").
    pub can_undo: bool,
    /// Something to tell: the source can't be opened, or the model changed.
    pub notice: Option<Notice>,
}

/// What a change of tab, task or model did.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RoleChangeOutcome {
    pub snapshot: Box<CreateSnapshot>,
    /// What it did to the images' roles, under the mode picker until dismissed (`RoleChangeNote`).
    pub note: Option<String>,
    /// It dropped inputs: offer Undo ("Draft replaced").
    pub can_undo: bool,
}

/// What adding files did: a line for the toast when some weren't added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AddOutcome {
    pub notice: Option<String>,
}

struct State {
    form: CreateForm,
    media: MediaState,
    estimates: EstimateCache,
    /// The provider generation `estimates` belong to.
    estimates_generation: u64,
    quoting: HashSet<String>,
    capabilities: Option<(u64, Capabilities)>,
    /// What the last replacement left, so an unedited one isn't offered back.
    untouched: Option<CreateForm>,
    /// The draft the last replacement lost.
    replaced: Option<CreateForm>,
    /// The source whose playback and thumbnails were asked for.
    prepared: Option<Url>,
    /// Goes up with every edit; a quote waits for it to settle.
    edits: u64,
    saves: u64,
    /// The style gallery and the account services of the connection.
    gallery: gallery::GalleryState,
    account: gallery::AccountState,
    link: links::LinkState,
}

/// The Create screen's video side.
pub struct CreateService {
    deps: CreateDeps,
    /// Create Character's requests and the sheets they made, for References › From Higgsfield › Characters.
    characters: std::sync::OnceLock<Arc<crate::images::characters::CharacterBuilder>>,
    drafts: CreateDraftStore,
    pricing: Option<Arc<Pricing>>,
    state: Mutex<State>,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for CreateService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CreateService").finish_non_exhaustive()
    }
}

impl CreateService {
    /// Opens with the saved draft, without the files that can no longer be read, or a fresh form from Settings.
    pub fn new(deps: CreateDeps) -> Arc<Self> {
        let drafts = CreateDraftStore::new(deps.layout.create_draft_path());
        let preferences = deps.preferences.get();
        let mut form = drafts.restore().unwrap_or_else(|| {
            CreateForm::new(
                preferences.default_mode(),
                preferences.default_resolution(),
                preferences.draft_resolution(),
            )
        });
        form.set_draft_resolution(preferences.draft_resolution());
        let pricing = Pricing::bundled()
            .inspect_err(|error| tracing::error!(%error, "can't read the bundled prices"))
            .ok()
            .map(Arc::new);
        Arc::new(Self {
            deps,
            characters: std::sync::OnceLock::new(),
            drafts,
            pricing,
            state: Mutex::new(State {
                form,
                media: MediaState::default(),
                estimates: EstimateCache::default(),
                estimates_generation: 0,
                quoting: HashSet::new(),
                capabilities: None,
                untouched: None,
                replaced: None,
                prepared: None,
                edits: 0,
                saves: 0,
                gallery: gallery::GalleryState::default(),
                account: gallery::AccountState::default(),
                link: links::LinkState::default(),
            }),
            changes: watch::Sender::new(0),
        })
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bumps whenever the snapshot changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn notify(&self) {
        let revision = {
            let mut state = self.locked();
            state.media.revision += 1;
            state.media.revision
        };
        self.changes.send_replace(revision);
    }

    /// The form as it is now.
    pub fn form(&self) -> CreateForm {
        self.locked().form.clone()
    }

    /// An edit of the form: counts as a change for the quotes and the draft file.
    fn edited(self: &Arc<Self>) {
        let saves = {
            let mut state = self.locked();
            state.edits += 1;
            state.saves += 1;
            state.saves
        };
        self.notify();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            if let Some(service) = weak.upgrade()
                && service.locked().saves == saves
            {
                service.save_now().await;
            }
        });
    }

    /// Writes the draft file now: on quit, and a moment after each change.
    pub async fn save_now(&self) {
        let form = self.form();
        let path = self.drafts.path().to_path_buf();
        let result = tokio::task::spawn_blocking(move || CreateDraftStore::new(path).save(Some(&form))).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "can't save the Create draft"),
            Err(error) => tracing::error!(%error, "can't save the Create draft"),
        }
    }

    // MARK: Context

    /// What the connection runs; cached per provider.
    async fn capabilities(&self) -> Option<Capabilities> {
        let provider = self.deps.provider.current()?;
        let generation = self.deps.provider.generation();
        if let Some((cached, capabilities)) = &self.locked().capabilities
            && *cached == generation
        {
            return Some(capabilities.clone());
        }
        match provider.capabilities().await {
            Ok(capabilities) if self.deps.provider.generation() == generation => {
                self.locked().capabilities = Some((generation, capabilities.clone()));
                Some(capabilities)
            }
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, "can't load capabilities for Create");
                None
            }
        }
    }

    /// Everything but the quotes, which are filled under the lock.
    async fn context(&self) -> CreateContext {
        self.context_and_spend().await.0
    }

    /// Today's spend toward `cap`: the jobs and the image charges. The error says why it can't be read.
    async fn committed_today(&self, cap: &SpendCap) -> Result<Decimal, String> {
        crate::settings::spending::committed_today(
            &self.deps.store,
            &self.deps.image_spend,
            cap.unit,
            OffsetDateTime::now_utc(),
            self.deps.utc_offset,
        )
        .await
    }

    /// The context, and why today's spend couldn't be read under a cap; the context then counts nothing spent.
    async fn context_and_spend(&self) -> (CreateContext, Option<String>) {
        let connection = self.deps.connection.state();
        let preferences = self.deps.preferences.get();
        let kind = if connection.is_connected() {
            connection.kind()
        } else {
            None
        };
        let capabilities = if kind.is_some() {
            self.capabilities().await
        } else {
            None
        };
        let cap = preferences.spend_cap(kind);
        let (committed_today, spend_error) = match &cap {
            Some(cap) => match self.committed_today(cap).await {
                Ok(committed) => (committed, None),
                Err(error) => {
                    tracing::error!(%error, "can't read today's spend");
                    (Decimal::ZERO, Some(error))
                }
            },
            None => (Decimal::ZERO, None),
        };
        let context = CreateContext {
            connection: kind,
            credits_left: connection.account_info().and_then(|info| info.credits),
            capabilities,
            pricing: self.pricing.clone(),
            estimates: EstimateCache::default(),
            committed_today,
            cap,
            unlimited_opt_in: preferences.uses_unlimited(),
        };
        (context, spend_error)
    }

    /// Fills `context` with the quotes of the current provider, dropping another provider's.
    fn with_estimates(&self, state: &mut State, mut context: CreateContext) -> CreateContext {
        let generation = self.deps.provider.generation();
        if state.estimates_generation != generation {
            state.estimates = EstimateCache::default();
            state.quoting.clear();
            state.estimates_generation = generation;
        }
        if let Some(capabilities) = &context.capabilities {
            state.form.set_unserved_modes(capabilities.unserved_modes.clone());
        }
        state
            .form
            .set_draft_resolution(self.deps.preferences.get().draft_resolution());
        context.estimates = state.estimates.clone();
        context
    }

    // MARK: Snapshot

    /// What the screen shows now. Asks for quotes the form still lacks, and prepares a restored source's media.
    pub async fn snapshot(self: &Arc<Self>) -> CreateSnapshot {
        let context = self.context().await;
        self.ensure_presets();
        self.ensure_constraints();
        let (view, quotes, source) = {
            let mut state = self.locked();
            let context = self.with_estimates(&mut state, context);
            let mut view = snapshot(&state.form, &context, &state.media);
            gallery::fill(&mut view, &state, &self.deps.preferences.get());
            view.link_download = state.link.view();
            let quotes = if context.is_connected() {
                let generate = state.form.specs_in(false, &context);
                let draft = if state.form.offers_draft() {
                    state.form.specs_in(true, &context)
                } else {
                    Vec::new()
                };
                let all: Vec<JobSpec> = generate.iter().chain(draft.iter()).cloned().collect();
                let missing: Vec<(String, JobSpec)> = state
                    .estimates
                    .missing(&all)
                    .into_iter()
                    .filter(|(key, _)| !state.quoting.contains(key))
                    .collect();
                for (key, _) in &missing {
                    state.quoting.insert(key.clone());
                }
                (!missing.is_empty()).then(|| (missing, generate, draft, state.edits))
            } else {
                None
            };
            let source = state.form.source().cloned().filter(|source| {
                let fresh = state.prepared.as_ref() != Some(&source.file_url);
                if fresh {
                    state.prepared = Some(source.file_url.clone());
                }
                fresh
            });
            (view, quotes, source)
        };
        if let Some((missing, generate, draft, edits)) = quotes {
            self.schedule_quotes(missing, generate, draft, edits);
        }
        if let Some(source) = source {
            self.prepare_source(source);
        }
        view
    }

    /// Asks the provider for `missing` once the form has been still for `QUOTE_DEBOUNCE`.
    fn schedule_quotes(
        self: &Arc<Self>,
        missing: Vec<(String, JobSpec)>,
        generate: Vec<JobSpec>,
        draft: Vec<JobSpec>,
        edits: u64,
    ) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(QUOTE_DEBOUNCE).await;
            let Some(service) = weak.upgrade() else {
                return;
            };
            let keys: Vec<String> = missing.iter().map(|(key, _)| key.clone()).collect();
            let release = |service: &Self| {
                let mut state = service.locked();
                for key in &keys {
                    state.quoting.remove(key);
                }
            };
            let provider = service.deps.provider.current();
            let generation = service.deps.provider.generation();
            let (Some(provider), true) = (provider, service.locked().edits == edits) else {
                release(&service);
                // A later snapshot asks again for the form as it is then.
                service.notify();
                return;
            };
            let quotes = futures::future::join_all(missing.iter().map(|(key, spec)| {
                let provider = provider.clone();
                async move { (key.clone(), provider.estimate(spec).await.ok()) }
            }))
            .await;
            let fresh: Vec<(String, Cost)> = quotes
                .into_iter()
                .filter_map(|(key, cost)| cost.map(|cost| (key, cost)))
                .collect();
            let recorded = !fresh.is_empty();
            {
                let mut state = service.locked();
                for key in &keys {
                    state.quoting.remove(key);
                }
                if state.estimates_generation == generation
                    && service.deps.provider.generation() == generation
                {
                    state.estimates.record(fresh, &generate, &draft);
                }
            }
            if recorded {
                service.notify();
            }
        });
    }

    // MARK: Edits

    /// Applies one edit and returns the snapshot after it.
    pub async fn edit(self: &Arc<Self>, edit: CreateEdit) -> Result<CreateSnapshot, CreateError> {
        {
            let mut state = self.locked();
            let mut form = state.form.clone();
            edit.apply(&mut form)?;
            if form.source().is_none() {
                state.prepared = None;
                state.media.playback = PlaybackView::default();
                state.media.thumbnails.clear();
            }
            state.form = form;
        }
        self.edited();
        Ok(self.snapshot().await)
    }

    /// A change of tab, task or model (`CreatePanel.recheckRoles`): applies it, then checks the images' roles again.
    /// One that drops inputs replaces the draft, so Undo brings them back.
    pub async fn change_roles(self: &Arc<Self>, edit: CreateEdit) -> Result<RoleChangeOutcome, CreateError> {
        let (note, can_undo) = {
            let mut state = self.locked();
            let previous = state.form.clone();
            let mut form = previous.clone();
            let mut applied = Ok(());
            let note = form.rechecking_roles(|form| applied = edit.apply(form));
            applied?;
            let mut can_undo = false;
            if form.drops_inputs(&previous) {
                let lost = replacement(&previous, &form, state.untouched.as_ref());
                can_undo = lost.is_some();
                if lost.is_some() {
                    state.replaced = lost;
                }
                state.untouched = Some(form.clone());
            }
            state.form = form;
            (note, can_undo)
        };
        self.edited();
        Ok(RoleChangeOutcome {
            snapshot: Box::new(self.snapshot().await),
            note,
            can_undo,
        })
    }

    /// Opens the video at `path` as the source: probed, then played and stripped in the background.
    pub async fn open_source(self: &Arc<Self>, path: &Path) -> Result<(), CreateError> {
        self.open_source_from(path, None).await
    }

    /// `open_source` for a file downloaded from `origin`, which a job that sends it whole can pass instead.
    async fn open_source_from(self: &Arc<Self>, path: &Path, origin: Option<Url>) -> Result<(), CreateError> {
        let info = self.deps.media.probe(path).await?;
        let mut video = source_video(path, &info)?;
        video.remote_origin = origin;
        {
            let mut state = self.locked();
            state.form.set_source(Some(video.clone()));
            state.media.playback = PlaybackView::default();
            state.media.thumbnails.clear();
            state.prepared = Some(video.file_url.clone());
        }
        self.edited();
        self.prepare_source(video);
        Ok(())
    }

    /// Dropped files: a video becomes the source, images become references.
    pub async fn add_files(self: &Arc<Self>, paths: &[PathBuf]) -> Result<AddOutcome, CreateError> {
        let (videos, others): (Vec<&PathBuf>, Vec<&PathBuf>) = paths.iter().partition(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
        });
        if let Some(video) = videos.first() {
            self.open_source(video).await?;
        }
        let images: Vec<PathBuf> = others
            .into_iter()
            .filter(|path| jutsu_media::frames::is_image(path))
            .cloned()
            .collect();
        if images.is_empty() {
            return Ok(AddOutcome::default());
        }
        self.add_references(&images)
    }

    /// Adds images as references in order, skipping any beyond the limit.
    pub fn add_references(self: &Arc<Self>, paths: &[PathBuf]) -> Result<AddOutcome, CreateError> {
        let images: Vec<LocalImage> = paths
            .iter()
            .map(|path| file_url(path).map(LocalImage::new))
            .collect::<Result<_, _>>()?;
        let skipped = {
            let mut state = self.locked();
            let mut fresh: Vec<&Url> = Vec::new();
            for image in &images {
                if state
                    .form
                    .references()
                    .iter()
                    .all(|old| old.file_url != image.file_url)
                    && !fresh.contains(&&image.file_url)
                {
                    fresh.push(&image.file_url);
                }
            }
            let fresh = fresh.len();
            fresh.saturating_sub(state.form.add_references(images.clone()))
        };
        self.edited();
        self.make_thumbnails(images);
        let notice = match skipped {
            0 => None,
            1 => Some("1 image wasn't added".to_owned()),
            count => Some(format!("{count} images weren't added")),
        };
        Ok(AddOutcome { notice })
    }

    /// Fills a frame slot with the image at `path`.
    pub fn set_frame(self: &Arc<Self>, slot: FrameSlot, path: &Path) -> Result<(), CreateError> {
        let image = LocalImage::new(file_url(path)?);
        self.locked().form.set_frame(slot, Some(image.clone()));
        self.edited();
        self.make_thumbnails(vec![image]);
        Ok(())
    }

    /// Saves a pasted image under `pasted/` and adds it as a reference.
    pub async fn paste_image(self: &Arc<Self>, bytes: Vec<u8>) -> Result<AddOutcome, CreateError> {
        let extension = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "jpg"
        } else if bytes.starts_with(b"\x89PNG") {
            "png"
        } else {
            return Err(CreateError::Media(
                "This image can't be read. Save it as JPEG or PNG and add it again.".to_owned(),
            ));
        };
        let folder = self.deps.layout.pasted_images_folder();
        let path = folder.join(format!(
            "Pasted {}.{extension}",
            Uuid::new_v4().to_string().to_uppercase()
        ));
        let written = path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::create_dir_all(&folder)?;
            std::fs::write(&written, bytes)
        })
        .await
        .map_err(|error| CreateError::Media(error.to_string()))??;
        self.add_references(&[path])
    }

    // MARK: Media

    /// Plays the source (the file through a hard link under the caches, or a proxy) and makes the trim strip.
    fn prepare_source(self: &Arc<Self>, video: SourceVideo) {
        let Ok(path) = video.file_url.to_file_path() else {
            return;
        };
        let folder = self.stage_folder(&path);
        let weak = Arc::downgrade(self);
        let url = video.file_url.clone();
        tokio::spawn(async move {
            let Some(service) = weak.upgrade() else {
                return;
            };
            let playback = service.playable(&video, &path, &folder).await;
            service.update_media(&url, |media| media.playback = playback);
            let thumbnails = service
                .deps
                .media
                .thumbnails(&path, THUMBNAIL_COUNT, &folder.join("strip"))
                .await;
            match thumbnails {
                Ok(files) => service.update_media(&url, |media| media.thumbnails = files),
                Err(error) => tracing::warn!(%error, "can't make the trim strip"),
            }
        });
    }

    /// `cache/create/<key>`: the files made for one source, keyed by its path, size and date.
    fn stage_folder(&self, path: &Path) -> PathBuf {
        let stamp = std::fs::metadata(path)
            .map(|metadata| {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|date| date.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_secs());
                format!("{}:{modified}", metadata.len())
            })
            .unwrap_or_default();
        let key = sha256_hex(&format!("{}|{stamp}", path.display()));
        self.deps
            .layout
            .cache_folder()
            .join("create")
            .join(key.get(..16).unwrap_or(&key))
    }

    async fn playable(self: &Arc<Self>, video: &SourceVideo, path: &Path, folder: &Path) -> PlaybackView {
        if video.plays_in_webview() {
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("mp4");
            let linked = folder.join(format!("source.{extension}"));
            let made = linked.exists()
                || std::fs::create_dir_all(folder)
                    .and_then(|()| std::fs::hard_link(path, &linked))
                    .is_ok();
            if made {
                return PlaybackView {
                    path: Some(linked.to_string_lossy().into_owned()),
                    progress: None,
                    error: None,
                };
            }
        }
        let proxy = folder.join("proxy.mp4");
        if proxy.exists() {
            return PlaybackView {
                path: Some(proxy.to_string_lossy().into_owned()),
                progress: None,
                error: None,
            };
        }
        let info = MediaInfo {
            duration: video.duration,
            pixel_size: video.pixel_size,
            codec: video.codec.clone(),
            frame_rate: 0.0,
            file_size: video.file_size,
            has_audio: true,
        };
        let url = video.file_url.clone();
        self.update_media(&url, |media| media.playback.progress = Some(0.0));
        let progress = self.progress_reporter(url);
        match self.deps.media.proxy(path, &proxy, &info, progress).await {
            Ok(()) => PlaybackView {
                path: Some(proxy.to_string_lossy().into_owned()),
                progress: None,
                error: None,
            },
            Err(error) => PlaybackView {
                path: None,
                progress: None,
                error: Some(error.to_string()),
            },
        }
    }

    /// Reports proxy progress in steps of 5%.
    fn progress_reporter(self: &Arc<Self>, url: Url) -> jutsu_core::Progress<f64> {
        let weak = Arc::downgrade(self);
        let last = Arc::new(Mutex::new(0.0_f64));
        Arc::new(move |fraction: f64| {
            {
                let mut shown = last.lock().unwrap_or_else(PoisonError::into_inner);
                if fraction - *shown < 0.05 {
                    return;
                }
                *shown = fraction;
            }
            if let Some(service) = weak.upgrade() {
                service.update_media(&url, |media| {
                    media.playback.progress = Some(fraction.clamp(0.0, 1.0));
                });
            }
        })
    }

    /// Changes the media state while `url` is still the source.
    fn update_media(&self, url: &Url, change: impl FnOnce(&mut MediaState)) {
        {
            let mut state = self.locked();
            if state.form.source().map(|source| &source.file_url) != Some(url) {
                return;
            }
            change(&mut state.media);
        }
        self.notify();
    }

    /// Small copies of the images for the list, under the caches.
    fn make_thumbnails(self: &Arc<Self>, images: Vec<LocalImage>) {
        let folder = self.deps.layout.cache_folder().join("create").join("images");
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            for image in images {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                if service.locked().media.images.contains_key(&image.file_url) {
                    continue;
                }
                let Ok(path) = image.file_url.to_file_path() else {
                    continue;
                };
                let output = folder.join(format!(
                    "{}.png",
                    sha256_hex(image.file_url.as_str()).get(..16).unwrap_or_default()
                ));
                if service.deps.media.image_thumbnail(&path, &output).await {
                    service
                        .locked()
                        .media
                        .images
                        .insert(image.file_url.clone(), output);
                    service.notify();
                }
            }
        });
    }

    // MARK: Submit

    /// Draft (`is_draft`) or Generate. Over the cap, or with no price under one, it asks first; call again with
    /// `approved`. The form stays as it is.
    pub async fn submit(self: &Arc<Self>, is_draft: bool, approved: bool) -> SubmitOutcome {
        {
            let mut state = self.locked();
            if state.media.is_submitting {
                return SubmitOutcome::Busy;
            }
            state.media.is_submitting = true;
        }
        self.notify();
        let outcome = self.decide_and_enqueue(is_draft, approved).await;
        self.locked().media.is_submitting = false;
        self.notify();
        outcome
    }

    /// Under a cap, today's spend must have been read. The prices the form still lacks are asked at once, so the
    /// decision uses the form's own price rather than none.
    async fn decide_and_enqueue(&self, is_draft: bool, approved: bool) -> SubmitOutcome {
        let (context, spend_error) = self.context_and_spend().await;
        if let Some(error) = spend_error {
            self.deps
                .connection
                .show_alert(UserMessage::new("Can't read today's spending", error, None));
            return SubmitOutcome::Failed;
        }
        self.quote_missing(&context).await;
        let (specs, parent_id, batch_id) = {
            let mut state = self.locked();
            let context = self.with_estimates(&mut state, context);
            let specs = match state.form.submit_decision(is_draft, &context) {
                SubmitDecision::Blocked(reason) => return SubmitOutcome::Blocked { reason },
                SubmitDecision::Confirm(_, confirmation) if !approved => {
                    return SubmitOutcome::Confirm { confirmation };
                }
                SubmitDecision::Confirm(specs, _) | SubmitDecision::Start(specs) => specs,
            };
            // Shared by the jobs of a batch, so the queue and the library group them.
            let batch_id = state.form.is_batch().then(Uuid::new_v4);
            (specs, state.form.parent_id(), batch_id)
        };
        self.enqueue(specs, parent_id, batch_id, approved).await
    }

    /// Asks the provider for the prices the form still lacks, without the debounce.
    async fn quote_missing(&self, context: &CreateContext) {
        if !context.is_connected() {
            return;
        }
        let Some(provider) = self.deps.provider.current() else {
            return;
        };
        let generation = self.deps.provider.generation();
        let (missing, generate, draft) = {
            let mut state = self.locked();
            let context = self.with_estimates(&mut state, context.clone());
            let generate = state.form.specs_in(false, &context);
            let draft = if state.form.offers_draft() {
                state.form.specs_in(true, &context)
            } else {
                Vec::new()
            };
            let all: Vec<JobSpec> = generate.iter().chain(draft.iter()).cloned().collect();
            (state.estimates.missing(&all), generate, draft)
        };
        if missing.is_empty() {
            return;
        }
        let quotes = futures::future::join_all(missing.iter().map(|(key, spec)| {
            let provider = provider.clone();
            async move { provider.estimate(spec).await.ok().map(|cost| (key.clone(), cost)) }
        }))
        .await;
        let fresh: Vec<(String, Cost)> = quotes.into_iter().flatten().collect();
        let mut state = self.locked();
        if state.estimates_generation == generation && self.deps.provider.generation() == generation {
            state.estimates.record(fresh, &generate, &draft);
        }
    }

    async fn enqueue(
        &self,
        specs: Vec<JobSpec>,
        parent_id: Option<Uuid>,
        batch_id: Option<Uuid>,
        approved: bool,
    ) -> SubmitOutcome {
        let kind = self.deps.connection.state().kind();
        let total = specs.len();
        let mut queued: Vec<Job> = Vec::with_capacity(total);
        for spec in specs {
            if let Some(preset) = spec.preset_id {
                self.record_preset_use(preset);
            }
            let options = EnqueueOptions {
                over_cap_approved: approved,
                parent_id,
                batch_id,
                ..EnqueueOptions::default()
            };
            match self.deps.engine.enqueue(spec, options).await {
                Ok(job) => queued.push(job),
                Err(error) => {
                    let message = user_message(&error, kind);
                    if queued.is_empty() {
                        if let Some(message) = message {
                            self.deps.connection.show_alert(message);
                        }
                        return SubmitOutcome::Failed;
                    }
                    self.deps
                        .connection
                        .show_alert(partly_queued(message, queued.len(), total));
                    break;
                }
            }
        }
        match queued_notice(&queued) {
            Some(notice) => SubmitOutcome::Queued { notice },
            None => SubmitOutcome::Failed,
        }
    }

    // MARK: Jobs from the Queue and Library

    async fn job(&self, id: Uuid) -> Result<Job, CreateError> {
        self.deps
            .store
            .job(id)
            .await
            .map_err(|error| CreateError::Media(error.to_string()))?
            .ok_or_else(|| CreateError::NotFound("This job is no longer in the queue.".to_owned()))
    }

    /// The job's source as the form takes it: probed again, kept on the account, or `None` when it can't be read.
    async fn reopened_source(&self, spec: &JobSpec) -> Option<SourceVideo> {
        let clip = spec.source.as_ref()?;
        let Ok(path) = clip.file_url.to_file_path() else {
            return Some(SourceVideo {
                file_url: clip.file_url.clone(),
                duration: clip.duration,
                pixel_size: clip.pixel_size,
                codec: String::new(),
                file_size: 0,
                remote_origin: clip.remote_origin.clone(),
            });
        };
        if !path.is_file() {
            return None;
        }
        let info = self.deps.media.probe(&path).await.ok()?;
        source_video(&path, &info).ok()
    }

    /// "Edit Inputs" and "Generate again": the job's inputs replace the form. A form that loses inputs can be had
    /// back with `undo_replacement`.
    pub async fn load_job(self: &Arc<Self>, id: Uuid) -> Result<LoadOutcome, CreateError> {
        let job = self.job(id).await?;
        Ok(self.load_spec(&job.spec, job.parent_id).await)
    }

    /// Edit… and Extend… on a finished result (`mode`): the result opens as the source of a Seedance job made from
    /// it, the way Edit Inputs loads a job.
    pub async fn open_follow_up(
        self: &Arc<Self>,
        id: Uuid,
        mode: jutsu_core::Mode,
    ) -> Result<LoadOutcome, CreateError> {
        let job = self.job(id).await?;
        let spec = super::follow_up::follow_up(&job, mode)
            .ok_or_else(|| CreateError::NotFound("The result is no longer on this computer.".to_owned()))?;
        Ok(self.load_spec(&spec, Some(job.id)).await)
    }

    async fn load_spec(self: &Arc<Self>, spec: &JobSpec, parent_id: Option<Uuid>) -> LoadOutcome {
        let video = self.reopened_source(spec).await;
        let lost_source = spec.source.is_some() && video.is_none();
        let offered = self.capabilities().await.map(|capabilities| capabilities.models);
        let (can_undo, fitted, source) = {
            let mut state = self.locked();
            let previous = state.form.clone();
            let mut form = previous.clone();
            form.load(spec, video.as_ref(), parent_id);
            let fitted = form.fit_model(offered.as_ref());
            let lost = replacement(&previous, &form, state.untouched.as_ref());
            let can_undo = lost.is_some();
            if lost.is_some() {
                state.replaced = lost;
            }
            state.untouched = Some(form.clone());
            state.prepared = form.source().map(|source| source.file_url.clone());
            state.media.playback = PlaybackView::default();
            state.media.thumbnails.clear();
            let source = form.source().cloned();
            state.form = form;
            (can_undo, fitted, source)
        };
        self.edited();
        if let Some(source) = source {
            self.prepare_source(source);
        }
        let images = self.form().references().to_vec();
        self.make_thumbnails(images);
        let notice = if lost_source {
            Some(Notice {
                title: "Can't open the source video".to_owned(),
                detail: "Add the video again to queue this job.".to_owned(),
            })
        } else {
            fitted
        };
        LoadOutcome { can_undo, notice }
    }

    /// Fills the form from a video template; a loaded source and its ranges stay. A model the connection doesn't
    /// offer runs on Genjutsu, and the notice says so.
    pub async fn apply_template(self: &Arc<Self>, template: &jutsu_core::JobTemplate) -> Option<Notice> {
        let offered = self.capabilities().await.map_or_else(
            || BTreeSet::from([VideoModel::GENJUTSU]),
            |capabilities| capabilities.models,
        );
        let notice = self.locked().form.apply_template(template, &offered);
        self.edited();
        self.make_thumbnails(template.references.clone());
        notice
    }

    /// A prompt picked from the history, or Clear prompt (`""`): it replaces the form's prompt. Returns whether
    /// Undo can bring the old one back.
    pub fn replace_prompt(self: &Arc<Self>, text: &str) -> bool {
        let can_undo = {
            let mut state = self.locked();
            let previous = state.form.clone();
            let mut form = previous.clone();
            text.clone_into(&mut form.prompt);
            let lost = replacement(&previous, &form, state.untouched.as_ref());
            let can_undo = lost.is_some();
            if lost.is_some() {
                state.replaced = lost;
            }
            state.untouched = Some(form.clone());
            state.form = form;
            can_undo
        };
        self.edited();
        can_undo
    }

    /// Brings back the draft the last replacement lost.
    pub fn undo_replacement(self: &Arc<Self>) {
        let source = {
            let mut state = self.locked();
            let Some(previous) = state.replaced.take() else {
                return;
            };
            state.untouched = None;
            state.form = previous;
            state.prepared = state.form.source().map(|source| source.file_url.clone());
            state.media.playback = PlaybackView::default();
            state.media.thumbnails.clear();
            state.form.source().cloned()
        };
        self.edited();
        if let Some(source) = source {
            self.prepare_source(source);
        }
    }

    /// "Render final": the draft's request again as a final, at 720p or above (Kling at Pro), queued at once; asks
    /// first over the cap.
    pub async fn render_final(self: &Arc<Self>, id: Uuid, approved: bool) -> SubmitOutcome {
        let job = match self.job(id).await {
            Ok(job) => job,
            Err(error) => {
                return SubmitOutcome::Blocked {
                    reason: error.to_string(),
                };
            }
        };
        let spec = final_spec(&job.spec, self.deps.preferences.get().default_resolution());
        let context = self.context().await;
        if !approved {
            let cost = match self.deps.provider.current() {
                Some(provider) => provider.estimate(&spec).await.ok(),
                None => None,
            }
            .or_else(|| context.cost(&spec));
            if let SubmitDecision::Confirm(_, confirmation) = SubmitDecision::budgeted(
                vec![spec.clone()],
                cost.as_ref(),
                context.committed_today,
                context.cap.as_ref(),
            ) {
                return SubmitOutcome::Confirm { confirmation };
            }
        }
        self.enqueue(vec![spec], Some(job.parent_id.unwrap_or(job.id)), None, approved)
            .await
    }
}

/// A run of jobs that failed partway (`BatchEnqueue.message`): how many were queued, with the failure's own
/// message and action; a failure with no alert of its own still says the rest weren't queued.
pub fn partly_queued(failure: Option<UserMessage>, queued: usize, total: usize) -> UserMessage {
    let mut message = failure.unwrap_or_else(|| UserMessage::new("", "The rest weren't queued.", None));
    message.title = format!("{queued} of {total} jobs queued");
    message
}

/// A draft's request as a final: not a draft, at the default resolution kept within 720p–1080p and fitted to the
/// model, or Kling's Pro. A model's own draft-to-final call (`Job.finalizeSpec`) isn't in `jutsu-core`.
pub fn final_spec(draft: &JobSpec, default_resolution: Resolution) -> JobSpec {
    let mut spec = draft.clone();
    spec.is_draft = false;
    spec.draft_job_id = None;
    spec.resolution = if spec.model.declares(VideoControl::Quality) {
        KlingQuality::Pro.resolution()
    } else {
        let wanted = default_resolution.clamp(Resolution::P720, Resolution::P1080);
        spec.model.fitted(wanted, spec.mode)
    };
    spec
}

fn file_url(path: &Path) -> Result<Url, CreateError> {
    Url::from_file_path(path).map_err(|()| CreateError::NotFound(format!("Can't open “{}”.", path.display())))
}

fn source_video(path: &Path, info: &MediaInfo) -> Result<SourceVideo, CreateError> {
    Ok(SourceVideo {
        file_url: file_url(path)?,
        duration: info.duration,
        pixel_size: info.pixel_size,
        codec: info.codec.clone(),
        file_size: info.file_size,
        remote_origin: None,
    })
}

/// The quotes `estimates` has for `spec`; for tests.
#[cfg(test)]
pub(crate) fn quoted(service: &CreateService, spec: &JobSpec) -> Option<Cost> {
    service
        .locked()
        .estimates
        .costs
        .get(&super::context::price_key(spec))
        .cloned()
}

/// What an agent's order is built from (`AgentService+Video.swift`): a fresh form with the person's defaults, never
/// the Create draft, the context Create prices in, and the files read as Create reads a dropped one.
impl CreateService {
    /// A fresh form with the person's defaults.
    pub fn agent_base_form(&self) -> CreateForm {
        let preferences = self.deps.preferences.get();
        CreateForm::new(
            preferences.default_mode(),
            preferences.default_resolution(),
            preferences.draft_resolution(),
        )
    }

    /// The model limits the app holds now.
    pub fn agent_constraints(&self) -> jutsu_core::ModelConstraintTable {
        self.locked().form.constraints()
    }

    /// The connection, its capabilities, prices and cap, as Create sees them.
    pub async fn agent_context(&self) -> CreateContext {
        self.context().await
    }

    /// The provider's price for `spec`; `None` without a provider or a quote.
    pub async fn agent_estimate(&self, spec: &JobSpec) -> Option<Cost> {
        let provider = self.deps.provider.current()?;
        provider.estimate(spec).await.ok()
    }

    /// The clip at `path`, probed as Create probes a dropped one.
    pub async fn agent_source(&self, path: &Path) -> Result<SourceVideo, CreateError> {
        let info = self.deps.media.probe(path).await?;
        source_video(path, &info)
    }

    /// The recording at `path`, read as Create reads a dropped one; `None` when it can't be read as audio.
    pub async fn agent_speech(&self, path: &Path) -> Result<Option<jutsu_core::LocalAudio>, CreateError> {
        let Some(duration) = self.deps.media.audio_duration(path).await else {
            return Ok(None);
        };
        Ok(Some(jutsu_core::LocalAudio {
            file_url: file_url(path)?,
            duration,
        }))
    }
}
