//! Turning the user's files into files the API accepts.

use async_trait::async_trait;
use url::Url;

use crate::models::{JobSpec, LocalAudio, LocalImage, OutputMedia, PixelSize, SourceClip};
use crate::ports::{PortError, Progress};
use crate::provider::PreparedFile;

/// Turns the user's files into files the API accepts. Results are cached by content, so preparing the same input
/// again is cheap.
#[async_trait]
pub trait MediaPreparing: Send + Sync {
    /// Trims the selected range, transcodes to MP4 H.264/AAC within the size limit, and upscales when the clip
    /// allows it.
    async fn prepare_video(
        &self,
        source: &SourceClip,
        progress: Progress<f64>,
    ) -> Result<PreparedFile, PortError>;
    /// Converts to JPEG or PNG and downscales oversized images.
    async fn prepare_image(&self, image: &LocalImage) -> Result<PreparedFile, PortError>;
    /// Passes MP3, WAV and M4A through untouched and converts any other audio to M4A (AAC).
    async fn prepare_audio(&self, audio: &LocalAudio) -> Result<PreparedFile, PortError>;
    /// Writes the downloaded `result` to `output` with its sound as `spec.audio` asks (`AudioPlan`), without
    /// re-encoding the video. Returns `false` and writes nothing when the result stays as it is.
    async fn restore_audio(&self, spec: &JobSpec, result: &Url, output: &Url) -> Result<bool, PortError>;
    /// The image's size as it shows, upright (its orientation applied); `None` when it can't be read.
    async fn image_size(&self, image: &LocalImage) -> Option<PixelSize>;
    /// A downloaded result's length and upright frame size, read from the file; `None` when it can't be read.
    async fn output_media(&self, file: &Url) -> Option<OutputMedia>;
}
