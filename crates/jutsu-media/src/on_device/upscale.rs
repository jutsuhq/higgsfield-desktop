//! Upscale on this computer (Swift's `SuperResolutionRenderer`): a video's frames taken out with ffmpeg, 4× by the
//! upscaler a few at a time, and encoded back at the target size with the source's sound; an image 4× directly. The
//! upscaler sits behind `FrameUpscaling`, so the pipeline runs in tests with a stand-in.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use image::{ImageFormat, RgbImage, imageops::FilterType};
use jutsu_core::{
    EnhanceCodec, OnDeviceOutput, OnDeviceProgress, OnDeviceUpscale, OnDeviceUpscaleTarget, PixelSize,
    Progress,
};
use tokio_util::sync::CancellationToken;

use super::raw::{Encode, FrameWriter, Sound};
use crate::error::MediaError;
use crate::probe::probe_streams;
use crate::tools::{FfmpegRun, FfmpegTools, os_args};

/// Makes pictures 4× their size: Real-ESRGAN, or a stand-in.
#[async_trait]
pub trait FrameUpscaling: Send + Sync {
    /// Why it can't run on this computer; `None` when it can, downloaded or not.
    fn unavailable_reason(&self) -> Option<String>;
    /// Whether its files are here, checked.
    fn is_installed(&self) -> bool;
    /// What its download weighs, in bytes; `None` without one for this computer.
    fn download_size(&self) -> Option<i64>;
    /// Downloads and unpacks it; `progress` is 0…1.
    async fn install(&self, progress: Progress<f64>, cancel: &CancellationToken) -> Result<(), MediaError>;
    /// Writes every PNG in `input` 4× its size into `output`, under the same names.
    async fn upscale(
        &self,
        input: &Path,
        output: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), MediaError>;
}

/// Frames handed to the upscaler at once: each 4× frame is a large PNG on disk until it is encoded.
const BATCH: usize = 8;

/// A folder of the run's own next to its result, removed with everything in it when dropped.
struct WorkFolder(PathBuf);

impl WorkFolder {
    fn new(output: &Path) -> Result<Self, MediaError> {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let folder = parent.join(format!(".ondevice-frames-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder)?;
        Ok(Self(folder))
    }

    fn sub(&self, name: &str) -> Result<PathBuf, MediaError> {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder)?;
        Ok(folder)
    }
}

impl Drop for WorkFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Downloads the upscaler first when it isn't here yet, reporting `DownloadingModel`.
async fn ensure_installed(
    upscaler: &dyn FrameUpscaling,
    progress: Option<&Progress<OnDeviceProgress>>,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    if let Some(reason) = upscaler.unavailable_reason() {
        return Err(MediaError::EngineUnavailableBecause(reason));
    }
    if upscaler.is_installed() {
        return Ok(());
    }
    let report = progress.cloned();
    let download: Progress<f64> = Arc::new(move |fraction| {
        if let Some(report) = &report {
            report(OnDeviceProgress::DownloadingModel(fraction));
        }
    });
    upscaler.install(download, cancel).await
}

/// `target`'s size for frames of `source`; an upscale that isn't larger, or needs more than 4×, isn't made.
fn output_size(target: OnDeviceUpscaleTarget, source: PixelSize) -> Result<PixelSize, MediaError> {
    target.output(source).ok_or(MediaError::EngineUnavailable)
}

/// Upscales the video at `source` to `target` into `output` as an MP4 with the source's sound. Failing or cancelling
/// leaves nothing at `output`.
pub async fn upscale_video(
    tools: &FfmpegTools,
    upscaler: &dyn FrameUpscaling,
    target: OnDeviceUpscaleTarget,
    source: &Path,
    output: &Path,
    progress: Option<Progress<OnDeviceProgress>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let result = upscale_video_frames(tools, upscaler, target, source, output, progress, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

async fn upscale_video_frames(
    tools: &FfmpegTools,
    upscaler: &dyn FrameUpscaling,
    target: OnDeviceUpscaleTarget,
    source: &Path,
    output: &Path,
    progress: Option<Progress<OnDeviceProgress>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let report = probe_streams(tools, source.as_os_str()).await?;
    let video = report.playable_video()?.clone();
    let size = video.display;
    if !OnDeviceUpscale::offers_video(size) {
        return Err(MediaError::EngineUnavailable);
    }
    let result_size = output_size(target, size)?;
    ensure_installed(upscaler, progress.as_ref(), cancel).await?;
    let work = WorkFolder::new(output)?;
    let frames = work.sub("frames")?;
    let duration = if video.time.duration > 0.0 {
        video.time.duration
    } else {
        report.duration
    };
    extract_frames(tools, source, &frames, cancel).await?;
    let mut names: Vec<OsString> = std::fs::read_dir(&frames)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    names.sort();
    if names.is_empty() {
        return Err(MediaError::UnsupportedFormat);
    }
    let frame_rate = if video.frame_rate >= 1.0 {
        video.frame_rate
    } else {
        names.len() as f64 / duration.max(f64::EPSILON)
    };
    let scaled = PixelSize::new(
        size.width * OnDeviceUpscale::SCALE,
        size.height * OnDeviceUpscale::SCALE,
    );
    let encode = Encode {
        size: scaled,
        output_size: result_size,
        frame_rate,
        codec: EnhanceCodec::of(result_size, video.codec == "avc1"),
    };
    let mut writer = FrameWriter::spawn(tools, encode, source, Sound::FromSource, output).await?;
    let batch_in = work.sub("in")?;
    let batch_out = work.sub("out")?;
    let total = names.len();
    if let Some(progress) = &progress {
        progress(OnDeviceProgress::Working(0.0));
    }
    for (index, batch) in names.chunks(BATCH).enumerate() {
        for name in batch {
            std::fs::rename(frames.join(name), batch_in.join(name))?;
        }
        upscaler.upscale(&batch_in, &batch_out, cancel).await?;
        for name in batch {
            let enlarged = batch_out.join(name);
            let pixels = read_rgb(&enlarged, scaled).await?;
            writer.write(&pixels, cancel).await?;
            let _ = std::fs::remove_file(&enlarged);
            let _ = std::fs::remove_file(batch_in.join(name));
        }
        if let Some(progress) = &progress {
            let done = (index * BATCH + batch.len()).min(total);
            progress(OnDeviceProgress::Working(done as f64 / total as f64));
        }
    }
    writer.finish(cancel).await?;
    Ok(OnDeviceOutput {
        pixel_size: result_size,
        duration,
        file_extension: "mp4".to_owned(),
    })
}

/// Every frame of the video as numbered PNGs in `folder`, upright.
async fn extract_frames(
    tools: &FfmpegTools,
    source: &Path,
    folder: &Path,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let mut args = os_args(["-i"]);
    args.push(source.as_os_str().to_owned());
    args.extend(os_args([
        "-map",
        "0:v:0",
        "-an",
        "-fps_mode",
        "passthrough",
        "-pix_fmt",
        "rgb24",
    ]));
    args.push(folder.join("%08d.png").into_os_string());
    FfmpegRun {
        args,
        output: None,
        duration: 0.0,
        progress: None,
        working_directory: None,
    }
    .run(tools, cancel)
    .await
}

/// The PNG at `path` as packed RGB of `size`.
async fn read_rgb(path: &Path, size: PixelSize) -> Result<Vec<u8>, MediaError> {
    let path = path.to_path_buf();
    let picture = tokio::task::spawn_blocking(move || image::open(&path).map(|picture| picture.to_rgb8()))
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?
        .map_err(|error| MediaError::ExportFailed(error.to_string()))?;
    if i64::from(picture.width()) != size.width || i64::from(picture.height()) != size.height {
        return Err(MediaError::ExportFailed(format!(
            "the upscaler made a {}×{} frame where {}×{} was due",
            picture.width(),
            picture.height(),
            size.width,
            size.height
        )));
    }
    Ok(picture.into_raw())
}

/// Upscales the image at `source` to `target` into `output` as a PNG, opaque: the engine takes no transparency.
/// Failing or cancelling leaves nothing at `output`.
pub async fn upscale_image(
    tools: &FfmpegTools,
    upscaler: &dyn FrameUpscaling,
    target: OnDeviceUpscaleTarget,
    source: &Path,
    output: &Path,
    progress: Option<Progress<OnDeviceProgress>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let result = upscale_image_file(tools, upscaler, target, source, output, progress, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

async fn upscale_image_file(
    tools: &FfmpegTools,
    upscaler: &dyn FrameUpscaling,
    target: OnDeviceUpscaleTarget,
    source: &Path,
    output: &Path,
    progress: Option<Progress<OnDeviceProgress>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let picture = crate::still::image_at(tools, source, 0)
        .await
        .ok_or(MediaError::UnreadableImage)?;
    let size = PixelSize::new(i64::from(picture.width()), i64::from(picture.height()));
    if !OnDeviceUpscale::offers_image(size) {
        return Err(MediaError::EngineUnavailable);
    }
    let result_size = output_size(target, size)?;
    ensure_installed(upscaler, progress.as_ref(), cancel).await?;
    if let Some(progress) = &progress {
        progress(OnDeviceProgress::Working(0.0));
    }
    let work = WorkFolder::new(output)?;
    let input = work.sub("in")?;
    let enlarged = work.sub("out")?;
    let opaque = image::DynamicImage::ImageRgba8(picture).to_rgb8();
    let first = input.join("image.png");
    let written = first.clone();
    tokio::task::spawn_blocking(move || opaque.save_with_format(&written, ImageFormat::Png))
        .await
        .map_err(|error| MediaError::Io(error.to_string()))?
        .map_err(|error| MediaError::ExportFailed(error.to_string()))?;
    upscaler.upscale(&input, &enlarged, cancel).await?;
    let scaled = PixelSize::new(
        size.width * OnDeviceUpscale::SCALE,
        size.height * OnDeviceUpscale::SCALE,
    );
    let pixels = read_rgb(&enlarged.join("image.png"), scaled).await?;
    let destination = output.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), MediaError> {
        let picture = RgbImage::from_raw(scaled.width as u32, scaled.height as u32, pixels)
            .ok_or(MediaError::UnreadableImage)?;
        let picture = if scaled == result_size {
            picture
        } else {
            image::imageops::resize(
                &picture,
                result_size.width as u32,
                result_size.height as u32,
                FilterType::Lanczos3,
            )
        };
        picture
            .save_with_format(&destination, ImageFormat::Png)
            .map_err(|error| MediaError::ExportFailed(error.to_string()))
    })
    .await
    .map_err(|error| MediaError::Io(error.to_string()))??;
    if let Some(progress) = &progress {
        progress(OnDeviceProgress::Working(1.0));
    }
    Ok(OnDeviceOutput {
        pixel_size: result_size,
        duration: 0.0,
        file_extension: "png".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::test_support::{Video, adding_tone, dir, tools, write_png};

    /// Makes each picture 4× with a plain resize, and counts what it was asked to do.
    #[derive(Default)]
    struct FakeUpscaler {
        installed: Mutex<bool>,
        unavailable: Option<String>,
        batches: Mutex<Vec<usize>>,
    }

    #[async_trait]
    impl FrameUpscaling for FakeUpscaler {
        fn unavailable_reason(&self) -> Option<String> {
            self.unavailable.clone()
        }

        fn is_installed(&self) -> bool {
            *self.installed.lock().unwrap()
        }

        fn download_size(&self) -> Option<i64> {
            Some(1000)
        }

        async fn install(
            &self,
            progress: Progress<f64>,
            _cancel: &CancellationToken,
        ) -> Result<(), MediaError> {
            progress(0.5);
            progress(1.0);
            *self.installed.lock().unwrap() = true;
            Ok(())
        }

        async fn upscale(
            &self,
            input: &Path,
            output: &Path,
            _cancel: &CancellationToken,
        ) -> Result<(), MediaError> {
            let mut count = 0;
            for entry in std::fs::read_dir(input).unwrap() {
                let path = entry.unwrap().path();
                let picture = image::open(&path).unwrap().to_rgb8();
                let bigger = image::imageops::resize(
                    &picture,
                    picture.width() * 4,
                    picture.height() * 4,
                    FilterType::Nearest,
                );
                bigger.save(output.join(path.file_name().unwrap())).unwrap();
                count += 1;
            }
            self.batches.lock().unwrap().push(count);
            Ok(())
        }
    }

    fn recorder() -> (Arc<Mutex<Vec<OnDeviceProgress>>>, Progress<OnDeviceProgress>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        (log, Arc::new(move |value| sink.lock().unwrap().push(value)))
    }

    #[tokio::test]
    async fn a_video_is_downloaded_for_then_upscaled_in_batches_with_its_sound() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let silent = Video::new(2)
            .size(64, 36)
            .rate(5)
            .write(&tools, folder.path())
            .await;
        let source = adding_tone(&tools, &silent, folder.path()).await;
        let output = folder.path().join("big.partial");
        let upscaler = FakeUpscaler::default();
        let (log, progress) = recorder();

        let result = upscale_video(
            &tools,
            &upscaler,
            OnDeviceUpscaleTarget::Full,
            &source,
            &output,
            Some(progress),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(result.pixel_size, PixelSize::new(256, 144));
        assert_eq!(result.file_extension, "mp4");
        assert_eq!(*upscaler.batches.lock().unwrap(), vec![8, 2]);
        let log = log.lock().unwrap().clone();
        assert_eq!(log.first(), Some(&OnDeviceProgress::DownloadingModel(0.5)));
        assert_eq!(log.last(), Some(&OnDeviceProgress::Working(1.0)));
        let info = crate::probe::probe(&tools, &output).await.unwrap();
        assert_eq!(info.pixel_size, PixelSize::new(256, 144));
        assert!(info.has_audio);
        // Nothing of the run's own is left next to the result.
        let left: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".ondevice-frames-")
            })
            .collect();
        assert!(left.is_empty());
    }

    #[tokio::test]
    async fn a_delivery_size_scales_the_four_times_frames_down() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(1)
            .size(64, 36)
            .rate(5)
            .write(&tools, folder.path())
            .await;
        let output = folder.path().join("delivery.partial");
        let upscaler = FakeUpscaler {
            installed: Mutex::new(true),
            ..FakeUpscaler::default()
        };

        let result = upscale_video(
            &tools,
            &upscaler,
            OnDeviceUpscaleTarget::Factor(2.0),
            &source,
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(result.pixel_size, PixelSize::new(128, 72));
        let info = crate::probe::probe(&tools, &output).await.unwrap();
        assert_eq!(info.pixel_size, PixelSize::new(128, 72));
    }

    #[tokio::test]
    async fn an_image_comes_out_as_an_opaque_png_at_its_target() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = write_png(folder.path(), 40, 30);
        let output = folder.path().join("image.partial");
        let upscaler = FakeUpscaler {
            installed: Mutex::new(true),
            ..FakeUpscaler::default()
        };

        let result = upscale_image(
            &tools,
            &upscaler,
            OnDeviceUpscaleTarget::Full,
            &source,
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(result.pixel_size, PixelSize::new(160, 120));
        assert_eq!(result.file_extension, "png");
        let written = image::ImageReader::open(&output)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!((written.width(), written.height()), (160, 120));
        assert!(!written.color().has_alpha());
    }

    #[tokio::test]
    async fn no_engine_here_fails_with_its_reason_and_leaves_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = write_png(folder.path(), 40, 30);
        let output = folder.path().join("none.partial");
        let upscaler = FakeUpscaler {
            unavailable: Some("Real-ESRGAN publishes no build for linux on aarch64.".to_owned()),
            ..FakeUpscaler::default()
        };

        let outcome = upscale_image(
            &tools,
            &upscaler,
            OnDeviceUpscaleTarget::Full,
            &source,
            &output,
            None,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(
            outcome.map(|_| ()).unwrap_err().to_string(),
            "Upscale on this computer can't run here: Real-ESRGAN publishes no build for linux on aarch64."
        );
        assert!(!output.exists());
    }
}
