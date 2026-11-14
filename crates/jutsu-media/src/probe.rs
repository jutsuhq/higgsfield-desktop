//! What a media file holds, read with `ffprobe` (Swift's `MediaProbe`).

use std::ffi::{OsStr, OsString};
use std::path::Path;

use jutsu_core::PixelSize;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::error::MediaError;
use crate::tools::{FfmpegTools, run_capture};

/// What the import screen shows of a video.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// Seconds.
    pub duration: f64,
    /// Display size: the stored size with the stream's rotation applied.
    pub pixel_size: PixelSize,
    /// FourCC of the video codec, e.g. `avc1` or `hvc1`.
    pub codec: String,
    /// Nominal frames per second.
    pub frame_rate: f64,
    /// Bytes; 0 for a remote file.
    pub file_size: i64,
    /// Whether it has an audio stream.
    pub has_audio: bool,
}

/// The streams of a file as the pipeline needs them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Probe {
    /// The container's duration in seconds; 0 when unknown.
    pub duration: f64,
    /// The first video stream that isn't a cover picture.
    pub video: Option<VideoStream>,
    /// The first audio stream.
    pub audio: Option<StreamTime>,
    /// ffprobe's name for the container, e.g. `mov,mp4,m4a,3gp,3g2,mj2` or `png_pipe`.
    pub format_name: String,
}

/// A video stream.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VideoStream {
    /// Upright size.
    pub display: PixelSize,
    /// FourCC.
    pub codec: String,
    /// Nominal frames per second; 0 when unknown.
    pub frame_rate: f64,
    /// Where and how long it runs.
    pub time: StreamTime,
}

/// Where a stream starts and how long it runs, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StreamTime {
    pub start: f64,
    pub duration: f64,
}

impl StreamTime {
    pub fn end(self) -> f64 {
        self.start + self.duration
    }
}

impl Probe {
    /// Whether the container is a still image's rather than a video's.
    pub fn is_still_image(&self) -> bool {
        self.format_name.ends_with("_pipe") || self.format_name == "image2"
    }

    /// The video stream of a playable video: one that isn't a still image and runs for some time.
    pub fn playable_video(&self) -> Result<&VideoStream, MediaError> {
        match &self.video {
            Some(video) if !self.is_still_image() && self.duration > 0.0 => Ok(video),
            _ => Err(MediaError::UnsupportedFormat),
        }
    }
}

/// Reads what the import screen shows. Fails with `MediaError::UnsupportedFormat` when there is no video stream
/// ffmpeg can decode, so a broken or non-video file fails here instead of mid-export.
pub async fn probe(tools: &FfmpegTools, path: &Path) -> Result<MediaInfo, MediaError> {
    let report = probe_streams(tools, path.as_os_str()).await?;
    let video = report.playable_video()?;
    let file_size = std::fs::metadata(path).map_or(0, |metadata| metadata.len() as i64);
    Ok(MediaInfo {
        duration: report.duration,
        pixel_size: video.display,
        codec: video.codec.clone(),
        frame_rate: video.frame_rate,
        file_size,
        has_audio: report.audio.is_some(),
    })
}

/// `probe` for a `file:` URL, or a remote one read by ffprobe, whose file size is reported as 0.
pub async fn probe_url(tools: &FfmpegTools, url: &Url) -> Result<MediaInfo, MediaError> {
    if let Ok(path) = url.to_file_path() {
        return probe(tools, &path).await;
    }
    let report = probe_streams(tools, OsStr::new(url.as_str())).await?;
    let video = report.playable_video()?;
    Ok(MediaInfo {
        duration: report.duration,
        pixel_size: video.display,
        codec: video.codec.clone(),
        frame_rate: video.frame_rate,
        file_size: 0,
        has_audio: report.audio.is_some(),
    })
}

/// Seconds of the sound in the file at `path`: an audio file, or a video's soundtrack. Fails with
/// `MediaError::UnsupportedFormat` when it has no audio stream ffmpeg can read.
pub async fn audio_duration(tools: &FfmpegTools, path: &Path) -> Result<f64, MediaError> {
    let report = probe_streams(tools, path.as_os_str()).await?;
    let duration = report.duration;
    if report.audio.is_none() || !duration.is_finite() || duration <= 0.0 {
        return Err(MediaError::UnsupportedFormat);
    }
    Ok(duration)
}

/// Every stream of the file; any failure to read it is `UnsupportedFormat`.
pub(crate) async fn probe_streams(tools: &FfmpegTools, input: &OsStr) -> Result<Probe, MediaError> {
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(input.to_owned());
    let cancel = CancellationToken::new();
    let output = unsupported_on_failure(run_capture(&tools.ffprobe, args, &cancel).await, &cancel)?;
    parse(&output).ok_or(MediaError::UnsupportedFormat)
}

/// ffprobe reports unreadable containers with assorted errors; callers only need to know the file can't be used.
/// Cancellation is passed through so a cancelled job isn't marked failed.
pub(crate) fn unsupported_on_failure<T>(
    result: Result<T, MediaError>,
    cancel: &CancellationToken,
) -> Result<T, MediaError> {
    match result {
        Ok(value) => Ok(value),
        Err(MediaError::Cancelled) => Err(MediaError::Cancelled),
        Err(_) if cancel.is_cancelled() => Err(MediaError::Cancelled),
        Err(_) => Err(MediaError::UnsupportedFormat),
    }
}

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Format {
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    codec_tag_string: Option<String>,
    profile: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    start_time: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Value>,
    #[serde(default)]
    tags: serde_json::Map<String, serde_json::Value>,
}

fn parse(json: &[u8]) -> Option<Probe> {
    let report: Report = serde_json::from_slice(json).ok()?;
    let format = report.format?;
    let duration = number(format.duration.as_deref()).unwrap_or(0.0);
    let mut video = None;
    let mut audio = None;
    for stream in &report.streams {
        match stream.codec_type.as_deref() {
            Some("video") if video.is_none() && !is_cover(stream) => {
                video = video_stream(stream, duration);
            }
            Some("audio") if audio.is_none() && stream.codec_name.is_some() => {
                audio = Some(stream_time(stream, duration));
            }
            _ => {}
        }
    }
    Some(Probe {
        duration,
        video,
        audio,
        format_name: format.format_name.unwrap_or_default(),
    })
}

fn is_cover(stream: &Stream) -> bool {
    stream
        .disposition
        .get("attached_pic")
        .and_then(serde_json::Value::as_i64)
        == Some(1)
}

fn video_stream(stream: &Stream, container_duration: f64) -> Option<VideoStream> {
    let codec_name = stream.codec_name.clone()?;
    let (width, height) = (stream.width?, stream.height?);
    if width <= 0 || height <= 0 {
        return None;
    }
    let turned = (rotation(stream).rem_euclid(180.0) - 90.0).abs() < 1e-9;
    let display = if turned {
        PixelSize::new(height, width)
    } else {
        PixelSize::new(width, height)
    };
    let frame_rate = rate(stream.avg_frame_rate.as_deref())
        .filter(|rate| *rate > 0.0)
        .or_else(|| rate(stream.r_frame_rate.as_deref()))
        .unwrap_or(0.0);
    Some(VideoStream {
        display,
        codec: four_cc(stream, &codec_name),
        frame_rate,
        time: stream_time(stream, container_duration),
    })
}

fn stream_time(stream: &Stream, container_duration: f64) -> StreamTime {
    let start = number(stream.start_time.as_deref()).unwrap_or(0.0).max(0.0);
    let duration = number(stream.duration.as_deref()).unwrap_or(container_duration - start);
    StreamTime { start, duration }
}

/// Degrees the picture is turned for display: the display matrix's, else an old `rotate` tag.
fn rotation(stream: &Stream) -> f64 {
    stream
        .side_data_list
        .iter()
        .find_map(|data| data.get("rotation").and_then(serde_json::Value::as_f64))
        .or_else(|| {
            stream
                .tags
                .get("rotate")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.parse().ok())
        })
        .unwrap_or(0.0)
}

/// The codec as a QuickTime FourCC, as AVFoundation reports it: the container's tag when it has a readable one,
/// else the usual tag for the codec.
fn four_cc(stream: &Stream, codec_name: &str) -> String {
    if let Some(tag) = &stream.codec_tag_string
        && tag.len() == 4
        && tag.chars().all(|c| c.is_ascii_graphic())
        && !tag.starts_with('[')
    {
        return tag.clone();
    }
    match codec_name {
        "h264" => "avc1".to_owned(),
        "hevc" => "hvc1".to_owned(),
        "prores" => match stream.profile.as_deref() {
            Some("Proxy") => "apco",
            Some("LT") => "apcs",
            Some("HQ") => "apch",
            Some("4444") => "ap4h",
            Some("4444XQ") => "ap4x",
            _ => "apcn",
        }
        .to_owned(),
        other => other.to_owned(),
    }
}

fn number(text: Option<&str>) -> Option<f64> {
    text?.trim().parse::<f64>().ok().filter(|value| value.is_finite())
}

fn rate(text: Option<&str>) -> Option<f64> {
    let text = text?;
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Video, dir, tools};

    #[tokio::test]
    async fn reads_video_properties() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).rate(30).write(&tools, folder.path()).await;

        let info = probe(&tools, &url).await.unwrap();

        assert!((info.duration - 6.0).abs() < 0.1, "{}", info.duration);
        assert_eq!(info.pixel_size, PixelSize::new(720, 480));
        assert_eq!(info.codec, "avc1");
        assert!((info.frame_rate - 30.0).abs() < 0.5);
        assert!(info.file_size > 0);
        assert!(!info.has_audio);
    }

    #[tokio::test]
    async fn rejects_undecodable_file() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = folder.path().join("clip.webm");
        std::fs::write(&url, "not a video container").unwrap();

        assert_eq!(probe(&tools, &url).await, Err(MediaError::UnsupportedFormat));
    }

    #[test]
    fn cancellation_is_not_reported_as_unsupported() {
        let running = CancellationToken::new();
        assert_eq!(
            unsupported_on_failure::<i32>(Err(MediaError::Cancelled), &running),
            Err(MediaError::Cancelled)
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            unsupported_on_failure::<i32>(Err(MediaError::ExportFailed("load failed".into())), &cancelled),
            Err(MediaError::Cancelled)
        );
        assert_eq!(
            unsupported_on_failure::<i32>(Err(MediaError::ExportFailed("load failed".into())), &running),
            Err(MediaError::UnsupportedFormat)
        );
    }

    #[tokio::test]
    async fn a_still_image_is_not_a_video() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let image = crate::test_support::write_png(folder.path(), 64, 64);
        assert_eq!(probe(&tools, &image).await, Err(MediaError::UnsupportedFormat));
    }

    #[test]
    fn a_turned_stream_shows_its_sides_swapped() {
        let json = br#"{"streams":[{"codec_type":"video","codec_name":"h264","codec_tag_string":"avc1",
            "width":1920,"height":1080,"avg_frame_rate":"30000/1001",
            "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]}],
            "format":{"format_name":"mov,mp4","duration":"2.5"}}"#;
        let report = parse(json).unwrap();
        let video = report.video.unwrap();
        assert_eq!(video.display, PixelSize::new(1080, 1920));
        assert!((video.frame_rate - 29.97).abs() < 0.01);
        assert!((video.time.duration - 2.5).abs() < f64::EPSILON);
    }
}
