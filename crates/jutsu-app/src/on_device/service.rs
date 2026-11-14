//! Enhance's runs on this computer as the app uses them (`AppEnvironment+OnDevice.swift`): ffmpeg and Real-ESRGAN do
//! the work, the catalog takes each result as a new version of the item it was started on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jutsu_core::{
    MaterialReference, OnDeviceAction, OnDeviceEngine, OnDeviceEnhancing, OutputSubfolder, PixelSize,
    SmoothMotion,
};
use jutsu_media::{FfmpegTools, FrameUpscaling, ModelDownloader, OnDeviceEnhancer, RealEsrganEngine};
use jutsu_store::{FileLayout, Store};
use serde::Serialize;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::agent::{AgentOnDeviceMedia, AgentOnDeviceRuns, AgentOnDeviceTarget};
use super::runs::{OnDeviceModelConsent, OnDeviceRequest, OnDeviceRun, OnDeviceRuns, Register};
use super::sheet::{OnDeviceFile, OnDeviceKind, OnDeviceSheet, OnDeviceSheetInput};
use super::text::{self, OnDeviceOffer};
use crate::connection::ConnectionStore;
use crate::library::item::LibraryItem;
use crate::library::media::{MediaRow, material_row};
use crate::preferences::PreferencesStore;

/// A run as a card, the result page, the Queue and the tray show it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OnDeviceRunRow {
    pub id: String,
    /// The library item it adds a version to.
    pub item_id: String,
    pub title: String,
    /// "Smooth Motion to 60 fps · 42%", "Downloading the model · 12%", or that it waits.
    pub progress: String,
    /// 0…1; `None` while it waits.
    pub fraction: Option<f64>,
}

impl OnDeviceRunRow {
    pub fn new(run: &OnDeviceRun) -> Self {
        Self {
            id: run.id.to_string(),
            item_id: run.request.source.material_id.to_string(),
            title: text::action_title(&run.request.action),
            progress: text::progress(run),
            fraction: run.state.fraction(),
        }
    }
}

/// How starting went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OnDeviceStart {
    Started {
        run_id: String,
    },
    /// Not started, for this reason: a run on the item already waits or runs, or the download isn't agreed to.
    Refused {
        reason: String,
    },
}

/// The engine folder Settings › Storage lists and removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStorage {
    pub folder: PathBuf,
    pub bytes: i64,
}

/// The file a run is made from, and what it is.
struct Target {
    source: MaterialReference,
    file: PathBuf,
    title: String,
    is_image: bool,
}

/// Enhance on this computer.
pub struct OnDeviceService {
    runs: Arc<OnDeviceRuns>,
    /// The runs agents started.
    agents: AgentOnDeviceRuns,
    enhancer: OnDeviceEnhancer,
    real_esrgan: Option<Arc<RealEsrganEngine>>,
    tools: Option<FfmpegTools>,
    store: Arc<Store>,
    layout: FileLayout,
    preferences: Arc<PreferencesStore>,
}

impl std::fmt::Debug for OnDeviceService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("OnDeviceService").finish_non_exhaustive()
    }
}

impl OnDeviceService {
    /// ffmpeg from the environment and Real-ESRGAN in the app data dir. Clears what runs before a crash or a quit
    /// left in the Videos folder of every output folder the app knows, before any run can start.
    pub fn new(store: Arc<Store>, layout: FileLayout, preferences: Arc<PreferencesStore>) -> Arc<Self> {
        let tools = FfmpegTools::from_environment();
        let real_esrgan = Arc::new(RealEsrganEngine::new(
            &layout.root,
            ModelDownloader::new(reqwest::Client::new()),
        ));
        let upscaler: Arc<dyn FrameUpscaling> = real_esrgan.clone();
        let enhancer = OnDeviceEnhancer::new(tools.clone(), upscaler);
        let engine: Arc<dyn OnDeviceEnhancing> = Arc::new(enhancer.clone());
        let service = Self::with_engine(store, layout, preferences, tools, enhancer, engine);
        let service = Arc::new(Self {
            real_esrgan: Some(real_esrgan),
            ..service
        });
        for folder in service.scratches() {
            OnDeviceRuns::remove_leftovers(&folder);
        }
        service
    }

    /// `engine` runs the work; `enhancer` says which engines this computer has and what the download weighs.
    pub fn with_engine(
        store: Arc<Store>,
        layout: FileLayout,
        preferences: Arc<PreferencesStore>,
        tools: Option<FfmpegTools>,
        enhancer: OnDeviceEnhancer,
        engine: Arc<dyn OnDeviceEnhancing>,
    ) -> Self {
        let catalog = store.clone();
        let register: Register = Arc::new(move |request: OnDeviceRequest, file: PathBuf, output| {
            let catalog = catalog.clone();
            Box::pin(async move {
                let url =
                    Url::from_file_path(&file).map_err(|()| "The result's file can't be read.".to_owned())?;
                // Not onto an item deleted meanwhile: the run takes its file back (`OnDeviceRuns`).
                catalog
                    .add_edited_version(
                        request.source,
                        url,
                        Some(output.output_media()),
                        false,
                        OffsetDateTime::now_utc(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
        });
        let runs = OnDeviceRuns::new(engine, OnDeviceModelConsent::new(preferences.clone()), register);
        Self {
            agents: AgentOnDeviceRuns::new(runs.clone()),
            runs,
            enhancer,
            real_esrgan: None,
            tools,
            store,
            layout,
            preferences,
        }
    }

    pub fn runs(&self) -> &Arc<OnDeviceRuns> {
        &self.runs
    }

    pub fn agents(&self) -> &AgentOnDeviceRuns {
        &self.agents
    }

    /// Why this computer can't run `engine`; `None` when it can.
    pub fn unavailable_reason(&self, engine: OnDeviceEngine) -> Option<String> {
        self.enhancer.unavailable_reason(engine)
    }

    /// Whether `id` is a video or image of the Library with its file here, as an agent's run would take it.
    pub async fn has_item(&self, id: Uuid) -> bool {
        self.target(id).await.is_ok()
    }

    /// The item an agent named, read as `enhance` on this computer needs it.
    pub async fn agent_target(&self, id: Uuid) -> Result<AgentOnDeviceTarget, String> {
        let target = self.target(id).await?;
        let kind = if target.is_image {
            OnDeviceKind::UpscaleImage
        } else {
            OnDeviceKind::SmoothMotion
        };
        let media = match self.read(&target.file, kind).await {
            OnDeviceFile::Video {
                duration,
                frame_rate,
                pixel_size,
            } => AgentOnDeviceMedia::Video {
                size: pixel_size,
                frame_rate,
                duration,
            },
            OnDeviceFile::Image { pixel_size, .. } => AgentOnDeviceMedia::Image(Some(pixel_size)),
            OnDeviceFile::Unreadable if target.is_image => AgentOnDeviceMedia::Image(None),
            OnDeviceFile::Unreadable => AgentOnDeviceMedia::UnreadableVideo,
        };
        let action = kind.action(SmoothMotion::Fps60);
        let request = self.request(target.source, target.file, action);
        Ok(AgentOnDeviceTarget {
            source: request.source,
            file: request.file,
            media,
            destination: if target.is_image {
                request.destination
            } else {
                request.scratch.clone()
            },
            scratch: request.scratch,
        })
    }

    /// Which runs a result's page offers for `item`.
    pub fn offer(&self, item: &LibraryItem) -> OnDeviceOffer {
        OnDeviceOffer::of(item, |engine| self.runs.supports(engine))
    }

    /// "Wait until Upscale 4× finishes" while a run on `material_id` waits or runs.
    pub fn busy(&self, material_id: Uuid) -> Option<String> {
        self.runs.run_for(material_id).as_ref().map(text::busy)
    }

    /// Whether a saved image of `size` is offered Upscale › On this computer.
    pub fn upscales_image(&self, size: Option<PixelSize>) -> bool {
        OnDeviceOffer::upscales_image(size, |engine| self.runs.supports(engine))
    }

    /// Whether the saved image `id` is offered Upscale on This Computer…, by the size its file says.
    pub async fn offers_image_upscale(&self, id: Uuid) -> bool {
        let Ok(target) = self.target(id).await else {
            return false;
        };
        if !target.is_image {
            return false;
        }
        let size = match self.read(&target.file, OnDeviceKind::UpscaleImage).await {
            OnDeviceFile::Image { pixel_size, .. } => Some(pixel_size),
            _ => None,
        };
        self.upscales_image(size)
    }

    /// The runs waiting or running, the first one running.
    pub fn run_rows(&self) -> Vec<OnDeviceRunRow> {
        self.runs.runs().iter().map(OnDeviceRunRow::new).collect()
    }

    /// The run on `material_id`, for its card and page.
    pub fn run_row(&self, material_id: Uuid) -> Option<OnDeviceRunRow> {
        self.runs.run_for(material_id).as_ref().map(OnDeviceRunRow::new)
    }

    pub fn cancel(&self, run_id: Uuid) {
        self.runs.cancel(run_id);
    }

    /// Cancels `id` when it is a run waiting or running here; `false` when it isn't one (a job, say).
    pub fn cancel_run(&self, id: Uuid) -> bool {
        let ours = self.runs.runs().iter().any(|run| run.id == id);
        if ours {
            self.runs.cancel(id);
        }
        ours
    }

    /// The runs as the tray panel lists them, and as the icon's progress counts them.
    pub fn tray_runs(&self) -> Vec<crate::system::tray::Run> {
        self.runs
            .runs()
            .iter()
            .map(|run| crate::system::tray::Run {
                id: run.id,
                kind: crate::system::tray::RunKind::OnDevice,
                title: text::action_title(&run.request.action),
                detail: text::progress(run),
                fraction: run.state.fraction(),
            })
            .collect()
    }

    /// Shows each failed run's reason in the window's alert, until the app quits.
    pub fn follow_failures(&self, connection: Arc<ConnectionStore>) {
        let runs = Arc::clone(&self.runs);
        let mut changes = runs.subscribe();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                if let Some(failure) = runs.take_failure() {
                    connection.show_alert(failure);
                }
            }
        });
    }

    /// The sheet of `kind` on the item `id`, its file read.
    pub async fn sheet(
        &self,
        id: Uuid,
        kind: OnDeviceKind,
        motion: Option<&str>,
    ) -> Result<OnDeviceSheet, String> {
        let target = self.target(id).await?;
        let file = self.read(&target.file, kind).await;
        let engine = kind.engine();
        Ok(OnDeviceSheet::new(&OnDeviceSheetInput {
            kind,
            item_title: target.title,
            file,
            motion: motion.and_then(SmoothMotion::from_raw),
            unavailable_reason: self.enhancer.unavailable_reason(engine),
            needs_consent: self.runs.needs_consent(engine),
            download_size: self.enhancer.download_size(),
        }))
    }

    /// Starts `kind` on the item `id`'s current version. `agrees`: the person ticked the download agreement, which
    /// is remembered for the engine.
    pub async fn start(
        &self,
        id: Uuid,
        kind: OnDeviceKind,
        motion: Option<&str>,
        agrees: bool,
    ) -> Result<OnDeviceStart, String> {
        let target = self.target(id).await?;
        let motion = motion
            .and_then(SmoothMotion::from_raw)
            .unwrap_or(SmoothMotion::Fps60);
        let action = kind.action(motion);
        if let Some(reason) = self.enhancer.unavailable_reason(action.engine()) {
            return Ok(OnDeviceStart::Refused { reason });
        }
        if self.runs.needs_consent(action.engine()) {
            if !agrees {
                return Ok(OnDeviceStart::Refused {
                    reason: text::model_notice(self.enhancer.download_size()),
                });
            }
            self.runs.agree(action.engine());
        }
        if target.is_image != (kind == OnDeviceKind::UpscaleImage) {
            return Err(text::UNREADABLE.to_owned());
        }
        let request = self.request(target.source, target.file, action);
        let run_id = Uuid::new_v4();
        if !self.runs.start(request, run_id) {
            let reason = self
                .busy(id)
                .unwrap_or_else(|| text::model_notice(self.enhancer.download_size()));
            return Ok(OnDeviceStart::Refused { reason });
        }
        Ok(OnDeviceStart::Started {
            run_id: run_id.to_string(),
        })
    }

    /// The result goes in the output folder's Videos, or Images for an image; it is written in Videos first under a
    /// hidden name, which `new` clears on the next launch after a crash.
    pub fn request(
        &self,
        source: MaterialReference,
        file: PathBuf,
        action: OnDeviceAction,
    ) -> OnDeviceRequest {
        let folder = self.output_folder();
        let videos = OutputSubfolder::Videos.path(&folder);
        let destination = if action.engine() == OnDeviceEngine::ImageSuperResolution {
            OutputSubfolder::Images.path(&folder)
        } else {
            videos.clone()
        };
        OnDeviceRequest {
            source,
            file,
            action,
            destination,
            scratch: videos,
        }
    }

    /// Real-ESRGAN's folder and what it takes, once it was downloaded.
    pub fn engine_storage(&self) -> Option<EngineStorage> {
        let engine = self.real_esrgan.as_ref()?;
        let bytes = engine.size_on_disk();
        (bytes > 0).then(|| EngineStorage {
            folder: engine.engines_folder().to_path_buf(),
            bytes,
        })
    }

    /// Deletes Real-ESRGAN; the next upscale downloads it again. Refused while a run waits or runs.
    pub fn remove_engine(&self) -> Result<(), String> {
        if !self.runs.runs().is_empty() {
            return Err("Wait until the runs on this computer finish.".to_owned());
        }
        let Some(engine) = self.real_esrgan.as_ref() else {
            return Ok(());
        };
        engine.remove().map_err(|error| error.to_string())
    }

    fn output_folder(&self) -> PathBuf {
        self.preferences
            .get()
            .output_folder
            .map_or_else(|| self.layout.default_output_folder.clone(), PathBuf::from)
    }

    /// The Videos folder of every output folder the app knows: a run's scratch is the one current when it started.
    fn scratches(&self) -> Vec<PathBuf> {
        let preferences = self.preferences.get();
        let mut folders = vec![self.layout.default_output_folder.clone(), self.output_folder()];
        folders.extend(
            preferences
                .previous_output_folders
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from),
        );
        folders.dedup();
        folders
            .iter()
            .map(|folder| OutputSubfolder::Videos.path(folder))
            .collect()
    }

    async fn target(&self, id: Uuid) -> Result<Target, String> {
        let gone = || "This result is no longer in the library.".to_owned();
        let material = self
            .store
            .material(id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(gone)?;
        let source = material.reference();
        match material_row(&material).ok_or_else(gone)? {
            MediaRow::Video(item) => Ok(Target {
                source,
                file: item.output_file().ok_or_else(|| text::UNREADABLE.to_owned())?,
                title: item.title(),
                is_image: false,
            }),
            MediaRow::LocalImage(image) => Ok(Target {
                source,
                file: image
                    .file_url
                    .to_file_path()
                    .map_err(|()| text::UNREADABLE.to_owned())?,
                title: image.title.clone().unwrap_or_else(|| image.prompt.clone()),
                is_image: true,
            }),
            MediaRow::RemoteImage(..) => Err(text::UNREADABLE.to_owned()),
        }
    }

    async fn read(&self, file: &Path, kind: OnDeviceKind) -> OnDeviceFile {
        let Some(tools) = self.tools.as_ref() else {
            return OnDeviceFile::Unreadable;
        };
        if kind == OnDeviceKind::UpscaleImage {
            let Some(pixel_size) = jutsu_media::still::size(tools, file).await else {
                return OnDeviceFile::Unreadable;
            };
            return OnDeviceFile::Image {
                pixel_size,
                has_alpha: jutsu_media::still::has_alpha(file),
            };
        }
        match jutsu_media::probe::probe(tools, file).await {
            Ok(info) => OnDeviceFile::Video {
                duration: info.duration,
                frame_rate: info.frame_rate,
                pixel_size: info.pixel_size,
            },
            Err(_) => OnDeviceFile::Unreadable,
        }
    }
}
