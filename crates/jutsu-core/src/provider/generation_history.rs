//! What an account generated, including outside the app.

use async_trait::async_trait;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::job::JobState;
use crate::models::{
    AspectRatio, Cost, Job, LocalAudio, LocalImage, Mode, PixelSize, Resolution, SourceClip, VideoModel,
};
use crate::ports::PortError;
use crate::provider::{ProviderKind, RemoteInputs, RemoteJob};
use crate::support::Seconds;
use crate::support::range::ClosedRange;

/// A provider that can list what its account generated, including outside the app.
#[async_trait]
pub trait GenerationHistory: Send + Sync {
    /// One page of finished videos the app can run again, newest first; `None` for the first page.
    async fn history(&self, after: Option<HistoryCursor>) -> Result<HistoryPage, PortError>;
}

/// Where the next page starts. Opaque: only the provider that returned it reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryCursor {
    /// The provider's value.
    pub value: String,
}

impl HistoryCursor {
    /// A cursor of `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self { value: value.into() }
    }
}

/// One page of the account's history.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPage {
    /// Only the items the app can show; a page may hold none while later pages still do.
    pub items: Vec<HistoryItem>,
    /// Finished image generations the app can show, newest first.
    pub images: Vec<HistoryImageItem>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}

/// A finished generation from the account's history, in the app's terms.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    /// The provider's job ID, as `RemoteJob::request_id` holds it for jobs the app submitted.
    pub id: String,
    /// The library item this becomes once imported; kept across reloads of the history.
    pub job_id: Uuid,
    /// The mode.
    pub mode: Mode,
    /// The model.
    pub model: VideoModel,
    /// When it was made.
    pub created_at: OffsetDateTime,
    /// Liked on the provider's side; the library's star for the item, kept in step with it.
    pub is_liked: bool,
    /// The result.
    pub output_url: Url,
    /// The output's length in seconds as the provider's params state it; `None` when it didn't say. Only a hint
    /// until the file is downloaded: the params can be wrong (a Seedance Edit said 854×480 and 5 s of a 480×854,
    /// 3.71 s file), so an import reads the file's own (`Job::output_media`).
    pub duration: Option<Seconds>,
    /// The output's frame size as the params state it.
    pub pixel_size: Option<PixelSize>,
    /// The resolution.
    pub resolution: Option<Resolution>,
    /// The prompt.
    pub prompt: String,
    /// The uploaded source video, reference images and speech, reusable by media ID while the provider keeps them.
    pub inputs: RemoteInputs,
    /// Seedance's native draft (`draft: true` in its params): imported as a draft that can be finalized at 1080p
    /// within seven days of `created_at` (`RemoteJob::is_native_draft`).
    pub is_native_draft: bool,
    /// The draft this generation finalized (`draft_job_id` in its params), kept on the imported job's spec
    /// (`JobSpec::draft_job_id`); `None` for any other.
    pub draft_job_id: Option<String>,
    /// The images are a start and an end frame (`JobSpec::references_are_frames`), as a clip made from frames sent
    /// them.
    pub references_are_frames: bool,
}

impl HistoryItem {
    /// An item with nothing but its identity, what made it and its output.
    pub fn new(
        id: impl Into<String>,
        mode: Mode,
        model: VideoModel,
        created_at: OffsetDateTime,
        output_url: Url,
        inputs: RemoteInputs,
    ) -> Self {
        Self {
            id: id.into(),
            job_id: Uuid::new_v4(),
            mode,
            model,
            created_at,
            is_liked: false,
            output_url,
            duration: None,
            pixel_size: None,
            resolution: None,
            prompt: String::new(),
            inputs,
            is_native_draft: false,
            draft_job_id: None,
            references_are_frames: false,
        }
    }

    /// The inputs as a job that reuses the uploaded media. A remote file can't be trimmed, so the whole source is
    /// selected. Without the source's probed length and frame size, the output's stand in. A generation made
    /// without a source video keeps the output's length. A speech is taken to be as long as the output without the
    /// model's tail (`VideoModelDescriptor::speech_tail`).
    pub fn spec(
        &self,
        source_duration: Option<Seconds>,
        source_size: Option<PixelSize>,
    ) -> crate::models::JobSpec {
        let length = source_duration.or(self.duration).unwrap_or(0.0).max(0.0);
        let size = source_size.or(self.pixel_size).unwrap_or_default();
        let clip = self
            .inputs
            .video
            .as_ref()
            .map(|video| SourceClip::new(video.url.clone(), length, size, ClosedRange::new(0.0, length)));
        let has_clip = clip.is_some();
        let mut spec = crate::models::JobSpec::new(
            self.mode,
            self.model.clone(),
            clip,
            self.resolution.unwrap_or(Resolution::P720),
        );
        spec.references = self
            .inputs
            .images
            .iter()
            .map(|image| LocalImage::new(image.url.clone()))
            .collect();
        spec.prompt.clone_from(&self.prompt);
        spec.is_draft = self.is_native_draft;
        spec.remote_inputs = Some(self.inputs.clone());
        spec.length = (!has_clip).then_some(length);
        spec.aspect_ratio = (!has_clip).then_some(AspectRatio::Auto);
        spec.references_are_frames = self.references_are_frames && !has_clip;
        spec.draft_job_id.clone_from(&self.draft_job_id);
        spec.speech = self.inputs.audio.as_ref().map(|audio| LocalAudio {
            file_url: audio.url.clone(),
            duration: (self.duration.unwrap_or(0.0) - self.model.descriptor().speech_tail).max(0.0),
        });
        spec
    }
}

impl Job {
    /// A finished generation from the account's history as a library item. Its output is downloaded separately,
    /// and its length and size are read from that file; the price isn't known, so it counts as nothing against the
    /// spend cap. A native draft keeps its mark, and its finalize window counts from the item's `created_at`, the
    /// only time history gives.
    pub fn importing(item: &HistoryItem, owner: Option<String>, imported_at: OffsetDateTime) -> Self {
        let mut job = Self::new(
            item.job_id,
            item.spec(None, None),
            ProviderKind::Account,
            Cost::credits(Decimal::ZERO, true),
            Uuid::new_v4(),
            item.created_at,
        );
        job.owner = owner;
        job.state = JobState::Done;
        job.progress = 1.0;
        job.inputs = Some(item.inputs.clone());
        let mut remote = RemoteJob::new(item.id.clone());
        remote.is_native_draft = item.is_native_draft;
        job.remote = Some(remote);
        job.charged_at = Some(item.created_at);
        job.finished_at = Some(item.created_at);
        job.imported_at = Some(imported_at);
        job
    }
}

/// A finished image generation from the account's history, in the app's terms. Unlike a video it never becomes a
/// job: the library shows it as an image, and downloads its file only when opened, referenced or compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryImageItem {
    /// The provider's generation ID.
    pub id: String,
    /// The library row this becomes; kept across reloads of the history.
    pub job_id: Uuid,
    /// The prompt.
    pub prompt: String,
    /// The server's model ID, as the history reports it; the app runs no image models by catalog.
    pub model: String,
    /// When it was made.
    pub created_at: OffsetDateTime,
    /// Liked on the provider's side; the library's star for the item.
    pub is_liked: bool,
    /// The image.
    pub output_url: Url,
    /// The output's frame size as the provider states it; `None` when it didn't say.
    pub pixel_size: Option<PixelSize>,
}

impl HistoryImageItem {
    /// "16:9" (`PixelSize::aspect_label`); `None` when the provider didn't state the size.
    pub fn aspect_ratio(&self) -> Option<String> {
        self.pixel_size.and_then(|size| size.aspect_label())
    }
}
