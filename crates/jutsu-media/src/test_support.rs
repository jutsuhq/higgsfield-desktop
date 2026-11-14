//! Fixtures generated per test instead of committed as binaries: grey clips piped into ffmpeg, `sine` tones and
//! solid images, in a temporary folder.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use image::{ImageEncoder, Rgba, RgbaImage};
use tokio::io::AsyncWriteExt;

use crate::tools::{EncodeQuality, FfmpegTools, command};

/// The tools from `FFMPEG` / `FFPROBE` or `PATH`; `None`, with a note, when they aren't there and the test is
/// skipped.
pub fn tools() -> Option<FfmpegTools> {
    let tools = FfmpegTools::from_environment();
    if tools.is_none() {
        eprintln!("skipped: no ffmpeg and ffprobe in FFMPEG / FFPROBE or on PATH");
    }
    tools
}

/// A new temporary folder, removed when dropped.
pub fn dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("jutsu-media-tests-")
        .tempdir()
        .unwrap()
}

/// A silent video of grey frames.
pub struct Video {
    pub width: i64,
    pub height: i64,
    pub seconds: i64,
    pub rate: i64,
    pub hevc_mov: bool,
    /// The grey level of each frame, by its index.
    pub shade: Box<dyn Fn(i64) -> u8 + Send + Sync>,
}

impl Video {
    /// 720×480 at 5 fps, fading through grey.
    pub fn new(seconds: i64) -> Self {
        Self {
            width: 720,
            height: 480,
            seconds,
            rate: 5,
            hevc_mov: false,
            shade: Box::new(|index| (30 + index % 200) as u8),
        }
    }

    pub fn size(mut self, width: i64, height: i64) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn rate(mut self, rate: i64) -> Self {
        self.rate = rate;
        self
    }

    pub fn shade(mut self, shade: impl Fn(i64) -> u8 + Send + Sync + 'static) -> Self {
        self.shade = Box::new(shade);
        self
    }

    pub fn hevc_mov(mut self) -> Self {
        self.hevc_mov = true;
        self
    }

    /// Writes it into `folder` and returns its path.
    pub async fn write(&self, tools: &FfmpegTools, folder: &Path) -> PathBuf {
        self.try_write(tools, folder).await.unwrap()
    }

    /// `write`, or `None` when this ffmpeg can't encode it (HEVC).
    pub async fn try_write(&self, tools: &FfmpegTools, folder: &Path) -> Option<PathBuf> {
        let extension = if self.hevc_mov { "mov" } else { "mp4" };
        let output = folder.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
        let mut args: Vec<String> = vec![
            "-hide_banner".into(),
            "-v".into(),
            "error".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "gray".into(),
            "-s".into(),
            format!("{}x{}", self.width, self.height),
            "-r".into(),
            self.rate.to_string(),
            "-i".into(),
            "pipe:0".into(),
        ];
        if self.hevc_mov {
            args.extend(["-c:v", "libx265", "-tag:v", "hvc1", "-pix_fmt", "yuv420p"].map(String::from));
        } else {
            let encoder = tools.h264_encoder().await.ok()?;
            args.extend(["-vf".to_owned(), encoder.upload_filter().to_owned()]);
            args.extend(encoder.codec_args(EncodeQuality::Medium, 1e6));
        }
        args.push(output.to_string_lossy().into_owned());
        let mut child = command(&tools.ffmpeg)
            .args(&args)
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        let frame_bytes = (self.width * self.height) as usize;
        for index in 0..self.seconds * self.rate {
            let frame = vec![(self.shade)(index); frame_bytes];
            if stdin.write_all(&frame).await.is_err() {
                break;
            }
        }
        drop(stdin);
        let status = child.wait().await.ok()?;
        status.success().then_some(output)
    }
}

async fn ffmpeg(tools: &FfmpegTools, args: &[String]) {
    let status = command(&tools.ffmpeg)
        .args(["-hide_banner", "-v", "error", "-y"])
        .args(args)
        .status()
        .await
        .unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

/// A 440 Hz mono tone of `seconds`, in the type `extension` names: AAC for m4a, 16-bit PCM otherwise.
pub async fn tone(tools: &FfmpegTools, folder: &Path, seconds: f64, extension: &str) -> PathBuf {
    let output = folder.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
    let codec = if extension == "m4a" { "aac" } else { "pcm_s16le" };
    ffmpeg(
        tools,
        &[
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("sine=frequency=440:sample_rate=44100:duration={seconds}"),
            "-ac".into(),
            "1".into(),
            "-c:a".into(),
            codec.into(),
            output.to_string_lossy().into_owned(),
        ],
    )
    .await;
    output
}

/// `video` with a 440 Hz AAC tone as long as the video, as a new MP4.
pub async fn adding_tone(tools: &FfmpegTools, video: &Path, folder: &Path) -> PathBuf {
    let seconds = crate::probe::probe(tools, video).await.unwrap().duration;
    let output = folder.join(format!("{}.mp4", uuid::Uuid::new_v4()));
    ffmpeg(
        tools,
        &[
            "-i".into(),
            video.to_string_lossy().into_owned(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("sine=frequency=440:sample_rate=44100:duration={seconds}"),
            "-map".into(),
            "0:v".into(),
            "-map".into(),
            "1:a".into(),
            "-c:v".into(),
            "copy".into(),
            "-c:a".into(),
            "aac".into(),
            output.to_string_lossy().into_owned(),
        ],
    )
    .await;
    output
}

/// The still formats the fixtures write.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Still {
    Png,
    Jpeg,
    Tiff,
}

/// A solid-colour image (0.2, 0.5, 0.8); a JPEG may carry an EXIF orientation.
pub fn make_image(folder: &Path, width: u32, height: u32, kind: Still, orientation: u16) -> PathBuf {
    let extension = match kind {
        Still::Png => "png",
        Still::Jpeg => "jpeg",
        Still::Tiff => "tiff",
    };
    let path = folder.join(format!("{}.{extension}", uuid::Uuid::new_v4()));
    let image = RgbaImage::from_pixel(width, height, Rgba([51, 128, 204, 255]));
    match kind {
        Still::Png => image.save(&path).unwrap(),
        Still::Tiff => image::DynamicImage::ImageRgba8(image)
            .to_rgb8()
            .save(&path)
            .unwrap(),
        Still::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
            let mut bytes = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
                .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
                .unwrap();
            if orientation != 1 {
                bytes = with_orientation(&bytes, orientation);
            }
            std::fs::write(&path, bytes).unwrap();
        }
    }
    path
}

/// A PNG of `width` × `height` in the fixture colour.
pub fn write_png(folder: &Path, width: u32, height: u32) -> PathBuf {
    make_image(folder, width, height, Still::Png, 1)
}

/// `jpeg` with an EXIF segment holding only `orientation`.
fn with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"II\x2a\x00\x08\x00\x00\x00");
    tiff.extend_from_slice(&1_u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112_u16.to_le_bytes());
    tiff.extend_from_slice(&3_u16.to_le_bytes());
    tiff.extend_from_slice(&1_u32.to_le_bytes());
    tiff.extend_from_slice(&u32::from(orientation).to_le_bytes());
    tiff.extend_from_slice(&0_u32.to_le_bytes());
    let mut segment = b"Exif\0\0".to_vec();
    segment.extend_from_slice(&tiff);
    let length = u16::try_from(segment.len() + 2).unwrap();
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&segment);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// The stored width and height of the image at `path`.
pub fn image_size(path: &Path) -> (u32, u32) {
    image::image_dimensions(path).unwrap()
}

/// The grey level of the frame shown at `seconds` (its green channel), exactly at that time.
pub async fn shade(tools: &FfmpegTools, video: &Path, seconds: f64) -> u8 {
    let frame = crate::frames::grab(tools, video, seconds, None).await.unwrap();
    let (width, height) = frame.dimensions();
    frame.get_pixel(width / 2, height / 2)[1]
}
