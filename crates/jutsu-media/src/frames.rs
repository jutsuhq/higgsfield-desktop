//! Exact still frames of a video (Swift's `FrameGrab`), trim-strip thumbnails (`ThumbnailStrip`) and the contact
//! sheet an agent checks a result with.

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use image::{ImageEncoder, Rgba, RgbaImage};
use jutsu_core::{PixelSize, unused_file_path};
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;
use crate::probe::probe_streams;
use crate::tools::{FfmpegTools, os_args, run_capture};

/// The longest side a frame is taken at: a 4K frame decoded whole is 33 MB, and an agent reads detail well below this.
pub const MAXIMUM_SIDE: u32 = 2048;
/// The height of a contact sheet's cells: a 16:9 frame becomes 640 × 360, where framing, faces and large text still
/// read, and a row of frames is a third as wide as the same frames at 1080p.
pub const SHEET_CELL_HEIGHT: u32 = 360;
/// The black between a contact sheet's cells, so two frames of the same scene side by side still read as two.
pub const SHEET_GAP: u32 = 8;
/// The box a trim-strip thumbnail fits in.
pub const THUMBNAIL_SIZE: (u32, u32) = (320, 320);

/// The extensions of image files, as `UTType` conformance to `public.image` names them.
const IMAGE_EXTENSIONS: [&str; 22] = [
    "jpg", "jpeg", "jpe", "png", "heic", "heif", "hif", "avif", "tif", "tiff", "gif", "bmp", "webp", "ico",
    "icns", "jxl", "psd", "tga", "dng", "cr2", "nef", "arw",
];

/// Whether `path` names an image rather than a video, by its extension.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

/// Seconds the video runs.
pub async fn duration(tools: &FfmpegTools, path: &Path) -> Result<f64, MediaError> {
    let report = probe_streams(tools, path.as_os_str()).await?;
    report.playable_video()?;
    Ok(report.duration)
}

/// The size a frame of `display` is taken at to fit `fit` (width, height), never upscaled.
fn fitted(display: PixelSize, fit: Option<(u32, u32)>) -> (u32, u32) {
    let (width, height) = (display.width.max(1) as f64, display.height.max(1) as f64);
    let scale = fit.map_or(1.0, |(max_width, max_height)| {
        (f64::from(max_width) / width)
            .min(f64::from(max_height) / height)
            .min(1.0)
    });
    let side = |value: f64| ((value * scale).round() as u32).max(1);
    (side(width), side(height))
}

/// The frame shown at `seconds` from the start of the video at `path`: the exact frame, not the nearest keyframe,
/// oriented for display and fitted into `fit` when given. A time at or past the end gives the last frame.
pub(crate) async fn grab(
    tools: &FfmpegTools,
    path: &Path,
    seconds: f64,
    fit: Option<(u32, u32)>,
) -> Result<RgbaImage, MediaError> {
    let report = probe_streams(tools, path.as_os_str()).await?;
    let video = report.playable_video()?;
    let (width, height) = fitted(video.display, fit);
    let frame_length = if video.frame_rate > 0.0 {
        1.0 / video.frame_rate
    } else {
        0.1
    };
    // Every frame of a short window ending at `seconds` is decoded, and the last one shown by then is kept.
    let time = seconds.max(0.0);
    let start = (time - 2.0 * frame_length).max(0.0);
    let selected = frames_from(tools, path, start, Some(time - start), (width, height)).await?;
    if let Some(frame) = selected.into_iter().last() {
        return Ok(frame);
    }
    // Nothing shows that early (the first frame starts later): the first frame after it.
    frames_from(tools, path, time, None, (width, height))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| MediaError::ExportFailed(format!("No frame at {seconds} s")))
}

/// The frames from `start` (exact) up to `start + until` (or only the first one) at `size`.
async fn frames_from(
    tools: &FfmpegTools,
    path: &Path,
    start: f64,
    until: Option<f64>,
    (width, height): (u32, u32),
) -> Result<Vec<RgbaImage>, MediaError> {
    let mut args: Vec<OsString> = os_args(["-hide_banner", "-nostdin", "-v", "error", "-ss"]);
    args.push(format!("{start:.6}").into());
    args.push("-i".into());
    args.push(path.as_os_str().to_owned());
    let scale = format!("scale={width}:{height}:flags=lanczos,format=rgba");
    let filter = match until {
        Some(until) => format!("select='lte(t\\,{:.6})',{scale}", until + 1e-6),
        None => scale,
    };
    args.extend(os_args(["-map", "0:v:0", "-vf"]));
    args.push(filter.into());
    if until.is_none() {
        args.extend(os_args(["-frames:v", "1"]));
    }
    args.extend(os_args(["-fps_mode", "passthrough", "-f", "rawvideo", "pipe:1"]));
    let bytes = run_capture(&tools.ffmpeg, args, &CancellationToken::new()).await?;
    let frame_bytes = (width * height * 4) as usize;
    Ok(bytes
        .chunks_exact(frame_bytes)
        .filter_map(|chunk| RgbaImage::from_raw(width, height, chunk.to_vec()))
        .collect())
}

/// The frames of the video at `times`, in seconds from its start, oriented for display and at most `MAXIMUM_SIDE` on
/// their longest side; the exact frame, not the nearest keyframe. Each goes to `each` with its index as soon as it is
/// decoded, so a caller that writes it out keeps none of them.
pub async fn frames(
    tools: &FfmpegTools,
    path: &Path,
    times: &[f64],
    cancel: &CancellationToken,
    mut each: impl FnMut(usize, RgbaImage) -> Result<(), MediaError>,
) -> Result<(), MediaError> {
    for (index, time) in times.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        let image = grab(tools, path, *time, Some((MAXIMUM_SIDE, MAXIMUM_SIDE))).await?;
        each(index, image)?;
    }
    Ok(())
}

/// `count` frames for the trim timeline, each from the middle of its slice of the video, oriented for display and no
/// larger than `THUMBNAIL_SIZE`.
pub async fn thumbnail_strip(
    tools: &FfmpegTools,
    path: &Path,
    count: usize,
    cancel: &CancellationToken,
) -> Result<Vec<RgbaImage>, MediaError> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let length = duration(tools, path).await?;
    let mut images = Vec::with_capacity(count);
    for index in 0..count {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        let time = length * (index as f64 + 0.5) / count as f64;
        images.push(grab(tools, path, time, Some(THUMBNAIL_SIZE)).await?);
    }
    Ok(images)
}

/// `image` scaled to `height` keeping its shape, for a contact sheet: a small copy to keep while the frames are taken,
/// instead of the frame itself. The image itself when it is no taller.
pub fn scaled(image: &RgbaImage, height: u32) -> Cow<'_, RgbaImage> {
    if image.height() <= height || height == 0 {
        return Cow::Borrowed(image);
    }
    let width =
        ((f64::from(image.width()) * f64::from(height) / f64::from(image.height())).round() as u32).max(1);
    Cow::Owned(image::imageops::resize(
        image,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// One image of `rows`, each frame scaled to `cell_height` keeping its shape, `gap` pixels apart on black: a row per
/// clip, a column per time. `None` when there is nothing to draw.
pub fn sheet(rows: &[Vec<RgbaImage>], cell_height: u32, gap: u32) -> Option<RgbaImage> {
    let scaled: Vec<Vec<(&RgbaImage, u32)>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|image| {
                    let width =
                        f64::from(image.width()) * f64::from(cell_height) / f64::from(image.height().max(1));
                    (image, (width.round() as u32).max(1))
                })
                .collect()
        })
        .collect();
    let columns = scaled.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return None;
    }
    // Each column as wide as its widest frame, so the same time lines up down the sheet.
    let widths: Vec<u32> = (0..columns)
        .map(|column| {
            scaled
                .iter()
                .filter_map(|row| row.get(column).map(|cell| cell.1))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let width = widths.iter().fold(gap, |total, column| total + column + gap);
    let height = gap + rows.len() as u32 * (cell_height + gap);
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
    for (row_index, row) in scaled.iter().enumerate() {
        let y = gap + row_index as u32 * (cell_height + gap);
        let mut x = gap;
        for (column, (image, cell_width)) in row.iter().enumerate() {
            let offset = (widths[column] - cell_width) / 2;
            let cell = image::imageops::resize(
                *image,
                *cell_width,
                cell_height,
                image::imageops::FilterType::Lanczos3,
            );
            image::imageops::overlay(&mut canvas, &cell, i64::from(x + offset), i64::from(y));
            x += widths[column] + gap;
        }
    }
    Some(canvas)
}

/// Writes `image` as a PNG at `path`, which must not exist yet: nothing is written over.
pub fn write_png(image: &RgbaImage, path: &Path) -> Result<(), MediaError> {
    let file = match std::fs::File::options().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(MediaError::FileExists(path.to_path_buf()));
        }
        Err(error) => return Err(error.into()),
    };
    let result = image::codecs::png::PngEncoder::new(std::io::BufWriter::new(file)).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    );
    result.map_err(|error| {
        let _ = std::fs::remove_file(path);
        MediaError::ExportFailed(format!("PNG: {error}"))
    })
}

/// What a clip is, for an error that says which one couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameRole {
    /// A video result or file.
    Video,
    /// An image.
    Image,
    /// The source clip a result's job used.
    SourceVideo,
}

/// One clip to take frames of: its file and where its times start.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameClip {
    /// The file.
    pub path: PathBuf,
    /// Seconds added to each time asked: the start of the range a source clip's job used.
    pub offset: f64,
    /// What the clip is.
    pub role: FrameRole,
}

impl FrameClip {
    /// A result or a file: an image by its extension, else a video.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let role = if is_image(&path) {
            FrameRole::Image
        } else {
            FrameRole::Video
        };
        Self {
            path,
            offset: 0.0,
            role,
        }
    }

    /// The source clip of a result, whose job used it from `offset` seconds on.
    pub fn source(path: impl Into<PathBuf>, offset: f64) -> Self {
        Self {
            path: path.into(),
            offset,
            role: FrameRole::SourceVideo,
        }
    }

    /// The source clip of a result on Higgsfield, read by ffmpeg from its https link; `None` for another scheme.
    pub fn remote_source(url: &url::Url, offset: f64) -> Option<Self> {
        (url.scheme() == "https").then(|| Self::source(url.as_str(), offset))
    }

    /// What its frames are named after: the file's name without its extension; a link's last path segment.
    pub fn name(&self) -> String {
        let text = self.path.to_string_lossy();
        if text.starts_with("https://")
            && let Ok(url) = url::Url::parse(&text)
        {
            let segment = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or_default();
            return Path::new(segment)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Why `write` wrote nothing more.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameGrabReason {
    /// The clip can't be read: why.
    Unreadable(String),
    /// `time`, in seconds from the start of the file, is past its end at `duration`.
    PastEnd {
        /// The time asked, offset included.
        time: f64,
        /// Where the clip ends.
        duration: f64,
    },
    /// A frame was read but couldn't be written: not the clip's fault.
    NotWritten(String),
}

/// Why `write` wrote nothing more, and for which clip.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}", self.describe())]
pub struct FrameGrabError {
    /// The clip.
    pub clip: FrameClip,
    /// Why.
    pub reason: FrameGrabReason,
}

impl FrameGrabError {
    fn describe(&self) -> String {
        let name = self.clip.name();
        match &self.reason {
            FrameGrabReason::Unreadable(why) => format!("{name} can't be read: {why}"),
            FrameGrabReason::PastEnd { time, duration } => format!(
                "{name} ends at {} s, before {} s",
                format_seconds(*duration),
                format_seconds(*time)
            ),
            FrameGrabReason::NotWritten(why) => format!("A frame of {name} couldn't be written: {why}"),
        }
    }
}

/// One frame `write` wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenFrame {
    /// The index of its clip in the clips asked for.
    pub clip: usize,
    /// Seconds from the start of the clip's file.
    pub time: f64,
    /// The PNG.
    pub file: PathBuf,
}

/// What `write` wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenFrames {
    /// In the order asked, the first clip's first.
    pub frames: Vec<WrittenFrame>,
    /// Every frame on one image, a row per clip and a column per time: written for several clips or several times.
    pub sheet: Option<PathBuf>,
}

/// Writes each clip's frame at each of `times` (plus the clip's offset) into `folder` as PNG files, and for several
/// clips or several times a contact sheet named after the first clip. Every clip is checked first, that it can be read
/// and, for a video, runs past every time, so nothing is decoded for a request that fails; then each frame is written
/// as soon as it is taken, under a name no file has, keeping only a small copy of it for the sheet. An image gives the
/// same frame at every time.
pub async fn write(
    tools: &FfmpegTools,
    clips: &[FrameClip],
    times: &[f64],
    folder: &Path,
) -> Result<WrittenFrames, FrameGrabError> {
    let mut opened = Vec::with_capacity(clips.len());
    for clip in clips {
        opened.push(open(tools, clip, times).await?);
    }
    let mut frames = Vec::new();
    let mut rows = Vec::new();
    for (index, (clip, is_image)) in clips.iter().zip(opened).enumerate() {
        let (written, row) = write_clip(tools, clip, is_image, index, times, folder).await?;
        frames.extend(written);
        rows.push(row);
    }
    let mut sheet_path = None;
    if let Some(main) = clips.first()
        && (rows.len() > 1 || times.len() > 1)
        && let Some(image) = sheet(&rows, SHEET_CELL_HEIGHT, SHEET_GAP)
    {
        let path = unused_file_path(&folder.join(format!("{} frames.png", main.name())), Path::exists);
        write_png(&image, &path).map_err(|error| FrameGrabError {
            clip: main.clone(),
            reason: FrameGrabReason::NotWritten(error.to_string()),
        })?;
        sheet_path = Some(path);
    }
    Ok(WrittenFrames {
        frames,
        sheet: sheet_path,
    })
}

/// Whether `clip` is an image, once it is known to be readable and, for a video, to run past every time.
async fn open(tools: &FfmpegTools, clip: &FrameClip, times: &[f64]) -> Result<bool, FrameGrabError> {
    if std::fs::File::open(&clip.path).is_err() {
        return Err(FrameGrabError {
            clip: clip.clone(),
            reason: FrameGrabReason::Unreadable(
                "it was moved or deleted, or Jutsu Studio may no longer read it".to_owned(),
            ),
        });
    }
    if is_image(&clip.path) {
        return Ok(true);
    }
    let length = duration(tools, &clip.path)
        .await
        .map_err(|error| unreadable(clip, &error))?;
    if let Some(late) = times
        .iter()
        .map(|time| time + clip.offset)
        .find(|time| *time > length)
    {
        return Err(FrameGrabError {
            clip: clip.clone(),
            reason: FrameGrabReason::PastEnd {
                time: late,
                duration: length,
            },
        });
    }
    Ok(false)
}

/// Writes the clip's frame at each time as soon as it is taken; returns them and their copies for the sheet.
async fn write_clip(
    tools: &FfmpegTools,
    clip: &FrameClip,
    is_image: bool,
    index: usize,
    times: &[f64],
    folder: &Path,
) -> Result<(Vec<WrittenFrame>, Vec<RgbaImage>), FrameGrabError> {
    let mut frames = Vec::new();
    let mut row = Vec::new();
    let mut write_one = |time: f64, image: &RgbaImage| -> Result<(), FrameGrabError> {
        let at = time + clip.offset;
        let name = format!("{} at {} s.png", clip.name(), format_seconds(at));
        let path = unused_file_path(&folder.join(name), Path::exists);
        write_png(image, &path).map_err(|error| FrameGrabError {
            clip: clip.clone(),
            reason: FrameGrabReason::NotWritten(error.to_string()),
        })?;
        frames.push(WrittenFrame {
            clip: index,
            time: at,
            file: path,
        });
        row.push(scaled(image, SHEET_CELL_HEIGHT).into_owned());
        Ok(())
    };
    if is_image {
        let still = crate::still::image_at(tools, &clip.path, MAXIMUM_SIDE)
            .await
            .ok_or_else(|| unreadable(clip, &MediaError::UnreadableImage))?;
        for time in times {
            write_one(*time, &still)?;
        }
    } else {
        for time in times {
            let image = grab(
                tools,
                &clip.path,
                time + clip.offset,
                Some((MAXIMUM_SIDE, MAXIMUM_SIDE)),
            )
            .await
            .map_err(|error| unreadable(clip, &error))?;
            write_one(*time, &image)?;
        }
    }
    Ok((frames, row))
}

fn unreadable(clip: &FrameClip, error: &MediaError) -> FrameGrabError {
    tracing::error!("Can't take frames of a clip: {error}");
    FrameGrabError {
        clip: clip.clone(),
        reason: FrameGrabReason::Unreadable(error.to_string()),
    }
}

/// Seconds with up to three fraction digits and grouped thousands, as Swift's `.number.precision(.fractionLength(0...3))`
/// writes them in English: "0", "1.5", "1,234.25".
pub fn format_seconds(seconds: f64) -> String {
    let text = format!("{:.3}", seconds.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction = fraction.trim_end_matches('0');
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let sign = if seconds < 0.0 && !text.trim_matches(['0', '.']).is_empty() {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Still, Video, dir, image_size, make_image, tools};

    #[test]
    fn seconds_read_as_swift_formats_them() {
        assert_eq!(format_seconds(0.0), "0");
        assert_eq!(format_seconds(1.5), "1.5");
        assert_eq!(format_seconds(2.0), "2");
        assert_eq!(format_seconds(1234.25), "1,234.25");
        assert_eq!(format_seconds(0.1234), "0.123");
    }

    #[tokio::test]
    async fn frames_are_the_exact_times_and_the_sheet_has_a_row_per_clip() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let video = Video::new(2).size(640, 360).write(&tools, folder.path()).await;
        assert!((duration(&tools, &video).await.unwrap() - 2.0).abs() < 0.01);
        let mut taken: Vec<RgbaImage> = Vec::new();
        frames(
            &tools,
            &video,
            &[0.0, 0.6, 1.4],
            &CancellationToken::new(),
            |index, image| {
                assert_eq!(index, taken.len());
                taken.push(image);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(
            taken.iter().map(RgbaImage::width).collect::<Vec<_>>(),
            [640, 640, 640]
        );
        let image = make_image(folder.path(), 360, 640, Still::Png, 1);
        assert!(is_image(&image));
        assert!(!is_image(&video));
        let still = crate::still::image_at(&tools, &image, 8192).await.unwrap();
        assert_eq!(still.dimensions(), (360, 640));

        // Each column is as wide as its widest frame: 640 at 360 high, the portrait still 203.
        let sheet = sheet(&[taken, vec![still]], 360, 8).unwrap();
        assert_eq!(sheet.width(), 8 + 3 * (640 + 8));
        assert_eq!(sheet.height(), 8 + 2 * (360 + 8));
        assert!(super::sheet(&[], SHEET_CELL_HEIGHT, SHEET_GAP).is_none());
    }

    #[test]
    fn a_source_on_higgsfield_is_read_from_its_https_link_and_named_by_its_file() {
        let url = url::Url::parse("https://cdn.example.com/u/1/source.clip.mp4?sig=a.b/c").unwrap();
        let clip = FrameClip::remote_source(&url, 1.5).unwrap();
        assert_eq!(clip.role, FrameRole::SourceVideo);
        assert_eq!(clip.path.to_string_lossy(), url.as_str());
        assert_eq!(clip.name(), "source.clip");
        let other = url::Url::parse("http://cdn.example.com/source.mp4").unwrap();
        assert!(FrameClip::remote_source(&other, 0.0).is_none());
    }

    #[tokio::test]
    async fn a_png_is_never_written_over_a_file() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let image = make_image(folder.path(), 64, 48, Still::Jpeg, 1);
        let still = crate::still::image_at(&tools, &image, 8192).await.unwrap();
        let png = folder.path().join("frame.png");
        write_png(&still, &png).unwrap();
        assert_eq!(image_size(&png), (64, 48));
        assert!(matches!(write_png(&still, &png), Err(MediaError::FileExists(_))));
        assert!(matches!(
            write_png(&still, &image),
            Err(MediaError::FileExists(_))
        ));
    }

    /// A frame is decoded at most `MAXIMUM_SIDE` long, and the sheet keeps only a copy at its cell height.
    #[tokio::test]
    async fn frames_are_bounded_and_the_sheets_copy_is_small() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let video = Video::new(1).size(2880, 1620).write(&tools, folder.path()).await;
        let mut sizes = Vec::new();
        frames(&tools, &video, &[0.5], &CancellationToken::new(), |_, image| {
            sizes.push(image.dimensions());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(sizes, [(2048, 1152)]);
        // A time at the very end, which frames lets through, is the last frame.
        frames(&tools, &video, &[1.0], &CancellationToken::new(), |_, _| Ok(()))
            .await
            .unwrap();
        let image = make_image(folder.path(), 1280, 720, Still::Png, 1);
        let still = crate::still::image_at(&tools, &image, 8192).await.unwrap();
        let small = scaled(&still, SHEET_CELL_HEIGHT);
        assert_eq!(small.dimensions(), (640, 360));
        assert!(matches!(scaled(&small, 720), Cow::Borrowed(_)));
    }

    #[tokio::test]
    async fn thumbnail_strip_returns_requested_count() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let url = Video::new(6).write(&tools, folder.path()).await;

        let images = thumbnail_strip(&tools, &url, 5, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(images.len(), 5);
        assert!(
            images
                .iter()
                .all(|image| image.width() <= 320 && image.height() <= 320)
        );
    }

    fn files(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn name(clip: &FrameClip, time: f64) -> String {
        format!("{} at {} s.png", clip.name(), format_seconds(time))
    }

    /// A source clip's times start where its job's range did, and its frames are named at those times.
    #[tokio::test]
    async fn a_source_clips_times_start_at_its_offset_and_the_sheet_comes_with_a_comparison() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let output = folder.path().join("out");
        std::fs::create_dir_all(&output).unwrap();
        let result = FrameClip::new(Video::new(2).size(320, 180).write(&tools, folder.path()).await);
        let source = FrameClip::source(
            Video::new(3).size(320, 180).write(&tools, folder.path()).await,
            1.0,
        );
        assert_eq!(result.role, FrameRole::Video);

        let written = write(&tools, &[result.clone(), source.clone()], &[0.0, 1.5], &output)
            .await
            .unwrap();
        assert_eq!(
            written.frames.iter().map(|frame| frame.clip).collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        assert_eq!(
            written.frames.iter().map(|frame| frame.time).collect::<Vec<_>>(),
            [0.0, 1.5, 1.0, 2.5]
        );
        assert_eq!(
            written
                .frames
                .iter()
                .map(|frame| frame.file.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            [
                name(&result, 0.0),
                name(&result, 1.5),
                name(&source, 1.0),
                name(&source, 2.5)
            ]
        );
        assert_eq!(
            written.sheet.unwrap().file_name().unwrap().to_string_lossy(),
            format!("{} frames.png", result.name())
        );
        assert_eq!(files(&output).len(), 5);

        // Asked again, nothing is written over: each file gets a new name.
        let again = write(&tools, std::slice::from_ref(&result), &[0.0], &output)
            .await
            .unwrap();
        assert_eq!(
            again.frames[0].file.file_name().unwrap().to_string_lossy(),
            format!("{} at 0 s 2.png", result.name())
        );
        // One clip at one time needs no sheet.
        assert!(again.sheet.is_none());
        assert_eq!(files(&output).len(), 6);
    }

    /// A time past a clip's end, counted from its offset, refuses the request before any frame is taken.
    #[tokio::test]
    async fn a_time_past_a_clips_end_writes_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let output = folder.path().join("out");
        std::fs::create_dir_all(&output).unwrap();
        let result = FrameClip::new(Video::new(2).size(320, 180).write(&tools, folder.path()).await);
        let source = FrameClip::source(result.path.clone(), 1.5);

        let error = write(&tools, &[result, source.clone()], &[0.0, 1.0], &output)
            .await
            .unwrap_err();
        assert_eq!(error.clip, source);
        let FrameGrabReason::PastEnd { time, duration } = error.reason else {
            panic!("{error:?}")
        };
        assert!((time - 2.5).abs() < 1e-9 && (duration - 2.0).abs() < 0.01);
        assert_eq!(files(&output), Vec::<String>::new());
    }

    /// The error names the clip that couldn't be read, so the agent knows which one to fix.
    #[tokio::test]
    async fn the_clip_that_cant_be_read_is_named() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let result = FrameClip::new(Video::new(1).size(320, 180).write(&tools, folder.path()).await);
        let broken = folder.path().join("broken.mp4");
        std::fs::write(&broken, "not a video").unwrap();
        let gone = FrameClip::source(folder.path().join("gone.mp4"), 0.0);

        for clip in [FrameClip::new(&broken), gone] {
            let error = write(&tools, &[result.clone(), clip.clone()], &[0.0], folder.path())
                .await
                .unwrap_err();
            assert!(matches!(error.reason, FrameGrabReason::Unreadable(_)));
            assert_eq!(error.clip, clip);
        }
        assert!(!files(folder.path()).iter().any(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        }));
    }

    /// An image has one frame whatever the time, and several times get a sheet.
    #[tokio::test]
    async fn an_image_gives_the_same_frame_at_every_time() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let image = FrameClip::new(make_image(folder.path(), 64, 48, Still::Png, 1));
        assert_eq!(image.role, FrameRole::Image);

        let written = write(&tools, std::slice::from_ref(&image), &[0.0, 30.0], folder.path())
            .await
            .unwrap();
        assert_eq!(
            written.frames.iter().map(|frame| frame.time).collect::<Vec<_>>(),
            [0.0, 30.0]
        );
        let sizes: Vec<_> = written
            .frames
            .iter()
            .map(|frame| image_size(&frame.file))
            .collect();
        assert_eq!(sizes, [(64, 48), (64, 48)]);
        assert!(written.sheet.is_some());
    }
}
