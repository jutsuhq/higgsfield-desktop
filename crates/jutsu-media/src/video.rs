//! Trim and transcode to MP4 H.264/AAC for upload (Swift's `VideoExport`), the re-encode at a bitrate that fits the
//! upload limit (`VideoReencoder`) and the silent preview cut (`PreviewExport`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jutsu_core::{ClosedRange, PixelSize, Progress};
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;
use crate::probe::probe_streams;
use crate::tools::{EncodeQuality, FfmpegRun, FfmpegTools, H264Encoder, os_args};

/// What to cut out of a source video and how big the result may be.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoRequest {
    /// The source file.
    pub source: PathBuf,
    /// Seconds from the start of the file.
    pub start: f64,
    /// Seconds from the start of the file.
    pub end: f64,
    /// Set only when the frame has to grow for object swap.
    pub upscaled_size: Option<PixelSize>,
    /// The most bytes the result may have.
    pub byte_limit: i64,
}

impl VideoRequest {
    fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// AAC's bitrate in a re-encode.
pub const AUDIO_BIT_RATE: i64 = 128_000;
/// The lowest video bitrate a re-encode uses, however long the clip.
pub const MINIMUM_VIDEO_BIT_RATE: i64 = 100_000;

/// The video bitrate that fits `duration` seconds into `byte_limit`: 10% of the budget is left for container overhead
/// and encoder overshoot.
pub fn video_bit_rate(duration: f64, byte_limit: i64, has_audio: bool) -> i64 {
    let bits_per_second = byte_limit as f64 * 8.0 * 0.9 / duration.max(1.0);
    let video = bits_per_second - if has_audio { AUDIO_BIT_RATE as f64 } else { 0.0 };
    (video as i64).max(MINIMUM_VIDEO_BIT_RATE)
}

/// The video filter chain that scales to `size` (filling it) and hands frames to `encoder`.
pub(crate) fn scale_filter(size: PixelSize, encoder: H264Encoder) -> String {
    format!(
        "scale={}:{}:flags=bicubic,setsar=1,{}",
        size.width,
        size.height,
        encoder.upload_filter()
    )
}

/// H.264 wants even sides.
pub(crate) fn even_floor(size: PixelSize) -> PixelSize {
    PixelSize::new((size.width / 2 * 2).max(2), (size.height / 2 * 2).max(2))
}

/// The frame size and rate of the source, and whether it has sound.
async fn source_shape(tools: &FfmpegTools, source: &Path) -> Result<(PixelSize, f64, bool), MediaError> {
    let report = probe_streams(tools, source.as_os_str()).await?;
    let video = report.playable_video()?;
    let rate = if video.frame_rate > 0.0 {
        video.frame_rate
    } else {
        30.0
    };
    Ok((video.display, rate, report.audio.is_some()))
}

/// The first pass: the stock high-quality H.264/AAC encode of the range, which stops at `byte_limit` the way
/// AVFoundation's `fileLengthLimit` does, so a result that didn't fit comes out short.
pub(crate) async fn export(
    tools: &FfmpegTools,
    request: &VideoRequest,
    output: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let (display, rate, _) = source_shape(tools, &request.source).await?;
    let size = request.upscaled_size.unwrap_or_else(|| even_floor(display));
    let mut args = transcode_args(tools, request, size, rate, EncodeQuality::Highest).await?;
    args.extend(os_args(["-fs".to_owned(), request.byte_limit.to_string()]));
    args.push(output.as_os_str().to_owned());
    FfmpegRun {
        args,
        output: Some(output),
        duration: request.duration(),
        progress,
        working_directory: None,
    }
    .run(tools, cancel)
    .await
}

/// The fallback when the first pass can't fit the size limit: decode and re-encode H.264/AAC at a bitrate computed
/// from the clip's length.
pub async fn reencode(
    tools: &FfmpegTools,
    request: &VideoRequest,
    output: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let (display, rate, has_audio) = source_shape(tools, &request.source).await?;
    let size = request.upscaled_size.unwrap_or_else(|| even_floor(display));
    let bit_rate = video_bit_rate(request.duration(), request.byte_limit, has_audio);
    let mut args = transcode_args(tools, request, size, rate, EncodeQuality::BitRate(bit_rate)).await?;
    args.push(output.as_os_str().to_owned());
    FfmpegRun {
        args,
        output: Some(output),
        duration: request.duration(),
        progress,
        working_directory: None,
    }
    .run(tools, cancel)
    .await
}

async fn transcode_args(
    tools: &FfmpegTools,
    request: &VideoRequest,
    size: PixelSize,
    rate: f64,
    quality: EncodeQuality,
) -> Result<Vec<OsString>, MediaError> {
    let encoder = tools.h264_encoder().await?;
    let mut args: Vec<OsString> = os_args(["-y"]);
    args.extend(os_args(encoder.input_args()));
    args.extend(os_args([
        "-ss".to_owned(),
        format!("{:.6}", request.start),
        "-t".to_owned(),
        format!("{:.6}", request.duration()),
        "-i".to_owned(),
    ]));
    args.push(request.source.as_os_str().to_owned());
    args.extend(os_args([
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
        "-vf",
    ]));
    args.push(scale_filter(size, encoder).into());
    args.extend(os_args(
        encoder.codec_args(quality, (size.width * size.height) as f64 * rate),
    ));
    args.extend(os_args([
        "-c:a".to_owned(),
        "aac".to_owned(),
        "-b:a".to_owned(),
        AUDIO_BIT_RATE.to_string(),
        "-ar".to_owned(),
        "44100".to_owned(),
        "-ac".to_owned(),
        "2".to_owned(),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
        "-f".to_owned(),
        "mp4".to_owned(),
    ]));
    Ok(args)
}

/// A silent H.264 cut of a result for sharing; the caller picks the range and size (`ShortPreview`). Only the video
/// goes in, so the file has no audio and messengers loop it like a GIF; `byte_limit` is a hard cap if an encode
/// overshoots.
pub async fn export_preview(
    tools: &FfmpegTools,
    source: &Path,
    range: ClosedRange<f64>,
    render_size: PixelSize,
    byte_limit: i64,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let (_, rate, _) = source_shape(tools, source).await?;
    let encoder = tools.h264_encoder().await?;
    let duration = (range.upper() - range.lower()).max(0.0);
    let mut args: Vec<OsString> = os_args(["-y"]);
    args.extend(os_args(encoder.input_args()));
    args.extend(os_args([
        "-ss".to_owned(),
        format!("{:.6}", range.lower()),
        "-t".to_owned(),
        format!("{duration:.6}"),
        "-i".to_owned(),
    ]));
    args.push(source.as_os_str().to_owned());
    args.extend(os_args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"]));
    args.push(scale_filter(render_size, encoder).into());
    args.extend(os_args(encoder.codec_args(
        EncodeQuality::Medium,
        (render_size.width * render_size.height) as f64 * rate,
    )));
    args.extend(os_args([
        "-fs".to_owned(),
        byte_limit.to_string(),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
        "-f".to_owned(),
        "mp4".to_owned(),
    ]));
    args.push(output.as_os_str().to_owned());
    FfmpegRun {
        args,
        output: Some(output),
        duration,
        progress: None,
        working_directory: None,
    }
    .run(tools, cancel)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepare::MediaPreparer;
    use crate::probe::probe;
    use crate::test_support::{Video, dir, tools};

    #[tokio::test]
    async fn reencoder_fits_byte_limit() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).write(&tools, folder.path()).await;
        let output = folder.path().join("reencoded.mp4");
        let request = VideoRequest {
            source: url,
            start: 1.0,
            end: 5.0,
            upscaled_size: Some(PixelSize::new(784, 524)),
            byte_limit: 2 * 1024 * 1024,
        };

        reencode(&tools, &request, &output, None, &CancellationToken::new())
            .await
            .unwrap();
        let info = probe(&tools, &output).await.unwrap();

        assert_eq!(info.codec, "avc1");
        assert_eq!(info.pixel_size, PixelSize::new(784, 524));
        assert!((info.duration - 4.0).abs() < 0.1, "{}", info.duration);
        assert!(info.file_size <= request.byte_limit);
    }

    #[test]
    fn bit_rate_leaves_room_for_audio_and_overhead() {
        let limit = MediaPreparer::FILE_LENGTH_LIMIT;
        let video = video_bit_rate(30.0, limit, true);
        let bytes = (video + AUDIO_BIT_RATE) as f64 * 30.0 / 8.0;
        assert!(bytes <= limit as f64 * 0.91);
        assert!(bytes >= limit as f64 * 0.85);
    }

    #[tokio::test]
    async fn a_preview_is_silent_and_at_its_size() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = crate::test_support::adding_tone(
            &tools,
            &Video::new(3).size(320, 240).write(&tools, folder.path()).await,
            folder.path(),
        )
        .await;
        let output = folder.path().join("preview.mp4");

        export_preview(
            &tools,
            &source,
            ClosedRange::new(0.5, 2.5),
            PixelSize::new(240, 180),
            1024 * 1024,
            &output,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let info = probe(&tools, &output).await.unwrap();

        assert!(!info.has_audio);
        assert_eq!(info.pixel_size, PixelSize::new(240, 180));
        assert!((info.duration - 2.0).abs() < 0.25, "{}", info.duration);
    }

    #[tokio::test]
    async fn a_cancelled_export_leaves_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(2).write(&tools, folder.path()).await;
        let output = folder.path().join("cancelled.mp4");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let request = VideoRequest {
            source: url,
            start: 0.0,
            end: 2.0,
            upscaled_size: None,
            byte_limit: MediaPreparer::FILE_LENGTH_LIMIT,
        };

        assert_eq!(
            reencode(&tools, &request, &output, None, &cancel).await,
            Err(MediaError::Cancelled)
        );
        assert!(!output.exists());
    }
}
