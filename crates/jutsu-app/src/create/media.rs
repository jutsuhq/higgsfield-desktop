//! What Create reads from media files through `jutsu-media`: the probe, the trim strip's thumbnails, a small H.264
//! proxy the webview can play, and reference thumbnails. Tests pass a fake.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use jutsu_core::{PixelSize, Progress};
use jutsu_media::{FfmpegTools, MediaInfo, frames, probe, still, video};
use tokio_util::sync::CancellationToken;

use super::CreateError;
use crate::queue::MissingFfmpeg;

/// The longest side of a stage proxy.
const PROXY_HEIGHT: i64 = 540;
/// About 2 Mbit/s: plenty for a preview.
const PROXY_BYTES_PER_SECOND: f64 = 250_000.0;
/// The longest side of a reference thumbnail.
const REFERENCE_THUMBNAIL: u32 = 320;

/// Create's media reads.
#[async_trait]
pub trait SourceMedia: Send + Sync {
    /// The source's length, frame, codec and size.
    async fn probe(&self, path: &Path) -> Result<MediaInfo, CreateError>;
    /// `count` frames across the clip, written as PNGs into `folder`, in order.
    async fn thumbnails(&self, path: &Path, count: usize, folder: &Path)
    -> Result<Vec<PathBuf>, CreateError>;
    /// A small H.264 MP4 of the whole clip at `output`; `progress` gets fractions.
    async fn proxy(
        &self,
        path: &Path,
        output: &Path,
        info: &MediaInfo,
        progress: Progress<f64>,
    ) -> Result<(), CreateError>;
    /// A small upright PNG of the image at `path`, at `output`; `false` when it can't be read.
    async fn image_thumbnail(&self, path: &Path, output: &Path) -> bool;
    /// Seconds a recording runs; `None` when no sound can be read from the file.
    async fn audio_duration(&self, path: &Path) -> Option<f64>;
}

/// ffmpeg found on the `PATH` (`FfmpegTools::from_environment`); every call fails with a clear message without it.
pub struct FfmpegSourceMedia {
    tools: Option<FfmpegTools>,
}

impl FfmpegSourceMedia {
    pub fn from_environment() -> Self {
        Self {
            tools: FfmpegTools::from_environment(),
        }
    }

    fn tools(&self) -> Result<&FfmpegTools, CreateError> {
        self.tools
            .as_ref()
            .ok_or_else(|| CreateError::Media(MissingFfmpeg::MESSAGE.to_owned()))
    }
}

fn media_error(error: &jutsu_media::MediaError) -> CreateError {
    CreateError::Media(error.to_string())
}

/// `size` scaled to at most `PROXY_HEIGHT` on its short side, even.
fn proxy_size(size: PixelSize) -> PixelSize {
    let short = size.width.min(size.height).max(1);
    if short <= PROXY_HEIGHT {
        return PixelSize::new((size.width / 2 * 2).max(2), (size.height / 2 * 2).max(2));
    }
    let even = |side: i64| ((side * PROXY_HEIGHT / short) / 2 * 2).max(2);
    PixelSize::new(even(size.width), even(size.height))
}

#[async_trait]
impl SourceMedia for FfmpegSourceMedia {
    async fn probe(&self, path: &Path) -> Result<MediaInfo, CreateError> {
        probe::probe(self.tools()?, path)
            .await
            .map_err(|error| media_error(&error))
    }

    async fn thumbnails(
        &self,
        path: &Path,
        count: usize,
        folder: &Path,
    ) -> Result<Vec<PathBuf>, CreateError> {
        let tools = self.tools()?;
        let images = frames::thumbnail_strip(tools, path, count, &CancellationToken::new())
            .await
            .map_err(|error| media_error(&error))?;
        std::fs::create_dir_all(folder)?;
        let mut written = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let file = folder.join(format!("{index}.png"));
            if !file.exists() {
                frames::write_png(image, &file).map_err(|error| media_error(&error))?;
            }
            written.push(file);
        }
        Ok(written)
    }

    async fn proxy(
        &self,
        path: &Path,
        output: &Path,
        info: &MediaInfo,
        progress: Progress<f64>,
    ) -> Result<(), CreateError> {
        let tools = self.tools()?;
        if let Some(folder) = output.parent() {
            std::fs::create_dir_all(folder)?;
        }
        #[allow(clippy::cast_possible_truncation, reason = "bytes of a preview")]
        let byte_limit = (info.duration.max(1.0) * PROXY_BYTES_PER_SECOND) as i64;
        let request = video::VideoRequest {
            source: path.to_path_buf(),
            start: 0.0,
            end: info.duration,
            upscaled_size: Some(proxy_size(info.pixel_size)),
            byte_limit,
        };
        video::reencode(tools, &request, output, Some(progress), &CancellationToken::new())
            .await
            .map_err(|error| media_error(&error))
    }

    async fn image_thumbnail(&self, path: &Path, output: &Path) -> bool {
        let Ok(tools) = self.tools() else {
            return false;
        };
        let Some(image) = still::image_at(tools, path, REFERENCE_THUMBNAIL).await else {
            return false;
        };
        if let Some(folder) = output.parent()
            && std::fs::create_dir_all(folder).is_err()
        {
            return false;
        }
        output.exists() || frames::write_png(&image, output).is_ok()
    }

    async fn audio_duration(&self, path: &Path) -> Option<f64> {
        probe::audio_duration(self.tools().ok()?, path).await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_is_at_most_540_on_its_short_side_and_even() {
        assert_eq!(proxy_size(PixelSize::new(1920, 1080)), PixelSize::new(960, 540));
        assert_eq!(proxy_size(PixelSize::new(1080, 1920)), PixelSize::new(540, 960));
        assert_eq!(proxy_size(PixelSize::new(641, 361)), PixelSize::new(640, 360));
    }
}
