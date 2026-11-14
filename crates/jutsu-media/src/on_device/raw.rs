//! Frames as raw RGB through ffmpeg's pipes: a decoder whose frames are read one at a time, and an encoder that takes
//! them and writes the MP4 of an on-device result, with the source's sound when it keeps it.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use jutsu_core::{EnhanceCodec, PixelSize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;
use crate::tools::{EncodeQuality, FfmpegTools, command, failure, os_args, tail};

/// Bits a pixel of each frame gets: about what the macOS benchmark's results needed to look like their sources (#13).
const BITS_PER_PIXEL: f64 = 0.12;

/// A running ffmpeg whose stdout gives frames of one size as packed RGB, top row first.
pub(crate) struct FrameReader {
    child: Child,
    stdout: ChildStdout,
    errors: JoinHandle<VecDeque<String>>,
    frame_bytes: usize,
}

impl FrameReader {
    /// Runs ffmpeg with `args` (everything before the output) and reads `size` frames from its stdout.
    pub(crate) fn spawn(
        tools: &FfmpegTools,
        args: Vec<OsString>,
        size: PixelSize,
    ) -> Result<Self, MediaError> {
        let mut command = command(&tools.ffmpeg);
        command
            .args(os_args(["-hide_banner", "-nostdin", "-v", "error"]))
            .args(args)
            .args(os_args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"]))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| MediaError::ExportFailed(format!("ffmpeg: {error}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| MediaError::ExportFailed("ffmpeg: no output".to_owned()))?;
        let errors = tokio::spawn(tail(child.stderr.take()));
        Ok(Self {
            child,
            stdout,
            errors,
            frame_bytes: frame_bytes(size),
        })
    }

    /// The next frame; `None` after the last one. A decoder that failed reports its error then.
    pub(crate) async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<Vec<u8>>, MediaError> {
        let mut frame = vec![0_u8; self.frame_bytes];
        let read = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(MediaError::Cancelled),
            read = read_full(&mut self.stdout, &mut frame) => read?,
        };
        if read == self.frame_bytes {
            return Ok(Some(frame));
        }
        let status = self.child.wait().await?;
        if status.success() {
            Ok(None)
        } else {
            let errors = (&mut self.errors).await.unwrap_or_default();
            Err(MediaError::ExportFailed(failure(&errors, status)))
        }
    }
}

/// Reads until `buffer` is full or the stream ends; the number of bytes read.
async fn read_full(stdout: &mut ChildStdout, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = stdout.read(&mut buffer[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

fn frame_bytes(size: PixelSize) -> usize {
    (size.width.max(0) * size.height.max(0) * 3) as usize
}

/// The sound an encode puts in next to the frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sound {
    /// None.
    Without,
    /// The source's first audio stream, copied when MP4 holds its codec, else made AAC.
    FromSource,
}

/// A running ffmpeg that encodes the frames written to it into an MP4.
pub(crate) struct FrameWriter {
    child: Child,
    stdin: Option<ChildStdin>,
    errors: JoinHandle<VecDeque<String>>,
}

/// What a `FrameWriter` encodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Encode {
    /// The frames' size as they are written.
    pub size: PixelSize,
    /// The result's size; ffmpeg scales the frames to it when it differs.
    pub output_size: PixelSize,
    pub frame_rate: f64,
    /// The codec the macOS app would write (`EnhanceCodec::of`).
    pub codec: EnhanceCodec,
}

impl FrameWriter {
    /// Encodes the frames into `output` at the macOS app's bit rate for the frame size (`FramePipeline.videoInput`),
    /// `sound` from `source`. HEVC is written with the first HEVC encoder that works here; with none, a frame that
    /// H.264 holds is written as H.264 (an LGPL ffmpeg has no software HEVC encoder).
    pub(crate) async fn spawn(
        tools: &FfmpegTools,
        encode: Encode,
        source: &Path,
        sound: Sound,
        output: &Path,
    ) -> Result<Self, MediaError> {
        let size = encode.output_size;
        let hevc = if encode.codec == EnhanceCodec::Hevc {
            hevc_encoder(tools).await
        } else {
            None
        };
        if encode.codec == EnhanceCodec::Hevc && hevc.is_none() && size.width.max(size.height) > H264_LIMIT {
            return Err(MediaError::ExportFailed(format!(
                "a {}×{} frame needs an HEVC encoder, and this computer's ffmpeg has none that works",
                size.width, size.height
            )));
        }
        let h264 = if hevc.is_none() {
            Some(tools.h264_encoder().await?)
        } else {
            None
        };
        let copies_sound = sound == Sound::FromSource && copies_audio(tools, source).await;
        let mut args = match (h264, hevc) {
            (Some(encoder), _) => os_args(encoder.input_args()),
            (None, Some(encoder)) => os_args(encoder.input_args()),
            (None, None) => Vec::new(),
        };
        args.extend(os_args([
            "-f".to_owned(),
            "rawvideo".to_owned(),
            "-pix_fmt".to_owned(),
            "rgb24".to_owned(),
            "-s".to_owned(),
            format!("{}x{}", encode.size.width, encode.size.height),
            "-framerate".to_owned(),
            encode.frame_rate.to_string(),
            "-i".to_owned(),
            "pipe:0".to_owned(),
        ]));
        if sound == Sound::FromSource {
            args.extend(os_args(["-i"]));
            args.push(source.as_os_str().to_owned());
            args.extend(os_args(["-map", "1:a:0?"]));
            args.extend(os_args(if copies_sound {
                vec!["-c:a", "copy"]
            } else {
                vec!["-c:a", "aac", "-b:a", "192k"]
            }));
        }
        let scale = if encode.size == size {
            String::new()
        } else {
            format!("scale={}:{}:flags=lanczos,", size.width, size.height)
        };
        let pixels_per_second = (size.width * size.height) as f64 * encode.frame_rate.max(1.0);
        match (h264, hevc) {
            (Some(encoder), _) => {
                let upload = encoder.upload_filter();
                args.extend(os_args([
                    "-map".to_owned(),
                    "0:v:0".to_owned(),
                    "-vf".to_owned(),
                    format!("{scale}{upload}"),
                ]));
                let bit_rate = ((pixels_per_second * BITS_PER_PIXEL) as i64).max(500_000);
                args.extend(os_args(
                    encoder.codec_args(EncodeQuality::BitRate(bit_rate), pixels_per_second),
                ));
            }
            (None, Some(encoder)) => {
                let upload = encoder.upload_filter();
                args.extend(os_args([
                    "-map".to_owned(),
                    "0:v:0".to_owned(),
                    "-vf".to_owned(),
                    format!("{scale}{upload}"),
                ]));
                let bit_rate = ((pixels_per_second * HEVC_BITS_PER_PIXEL) as i64).max(500_000);
                args.extend(os_args([
                    "-c:v".to_owned(),
                    encoder.name.to_owned(),
                    "-b:v".to_owned(),
                    bit_rate.to_string(),
                    "-tag:v".to_owned(),
                    "hvc1".to_owned(),
                ]));
            }
            (None, None) => {}
        }
        args.extend(os_args(["-movflags", "+faststart", "-f", "mp4", "-y"]));
        args.push(output.as_os_str().to_owned());
        Self::start(tools, args)
    }

    fn start(tools: &FfmpegTools, args: Vec<OsString>) -> Result<Self, MediaError> {
        let mut command = command(&tools.ffmpeg);
        command
            .args(os_args(["-hide_banner", "-nostdin", "-v", "error"]))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| MediaError::ExportFailed(format!("ffmpeg: {error}")))?;
        let stdin = child.stdin.take();
        let errors = tokio::spawn(tail(child.stderr.take()));
        Ok(Self { child, stdin, errors })
    }

    /// Hands one frame over.
    pub(crate) async fn write(&mut self, frame: &[u8], cancel: &CancellationToken) -> Result<(), MediaError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(MediaError::ExportFailed("ffmpeg: input closed".to_owned()));
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(MediaError::Cancelled),
            written = stdin.write_all(frame) => match written {
                Ok(()) => Ok(()),
                // The encoder stopped: its own error says why.
                Err(_) => Err(self.failure().await),
            },
        }
    }

    /// Closes the input and waits for the file to be written.
    pub(crate) async fn finish(mut self, cancel: &CancellationToken) -> Result<(), MediaError> {
        drop(self.stdin.take());
        let status = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(MediaError::Cancelled),
            status = self.child.wait() => status?,
        };
        if status.success() {
            Ok(())
        } else {
            let errors = (&mut self.errors).await.unwrap_or_default();
            Err(MediaError::ExportFailed(failure(&errors, status)))
        }
    }

    async fn failure(&mut self) -> MediaError {
        drop(self.stdin.take());
        match self.child.wait().await {
            Ok(status) => {
                let errors = (&mut self.errors).await.unwrap_or_default();
                MediaError::ExportFailed(failure(&errors, status))
            }
            Err(error) => error.into(),
        }
    }
}

/// Whether the source's first audio stream can go into an MP4 as it is (`FramePipeline.sound` passes it through when
/// the writer takes it).
async fn copies_audio(tools: &FfmpegTools, source: &Path) -> bool {
    let mut args = os_args([
        "-v",
        "error",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=codec_name",
        "-of",
        "csv=p=0",
    ]);
    args.push(source.as_os_str().to_owned());
    let Ok(output) = crate::tools::run_capture(&tools.ffprobe, args, &CancellationToken::new()).await else {
        return false;
    };
    matches!(
        String::from_utf8_lossy(&output).trim(),
        "aac" | "mp3" | "alac" | "ac3" | "eac3" | "opus" | "flac"
    )
}

/// The largest side H.264 holds (`EnhanceCodec::of`).
const H264_LIMIT: i64 = 4096;
/// HEVC's bits a pixel (`FramePipeline.videoInput`).
const HEVC_BITS_PER_PIXEL: f64 = 0.08;

/// An HEVC encoder ffmpeg offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HevcEncoder {
    pub name: &'static str,
}

impl HevcEncoder {
    /// In the order they are preferred.
    const PREFERENCE: [&'static str; 7] = [
        "libx265",
        "hevc_nvenc",
        "hevc_qsv",
        "hevc_amf",
        "hevc_vaapi",
        "hevc_videotoolbox",
        "hevc_mf",
    ];

    fn input_args(self) -> Vec<String> {
        if self.name == "hevc_vaapi" {
            vec!["-vaapi_device".into(), "/dev/dri/renderD128".into()]
        } else {
            Vec::new()
        }
    }

    fn upload_filter(self) -> &'static str {
        match self.name {
            "hevc_vaapi" => "format=nv12,hwupload",
            "hevc_qsv" => "format=nv12",
            _ => "format=yuv420p",
        }
    }
}

/// The first HEVC encoder this ffmpeg lists that encodes a short test clip here.
async fn hevc_encoder(tools: &FfmpegTools) -> Option<HevcEncoder> {
    let listing = crate::tools::run_capture(
        &tools.ffmpeg,
        os_args(["-hide_banner", "-encoders"]),
        &CancellationToken::new(),
    )
    .await
    .ok()?;
    let listing = String::from_utf8_lossy(&listing).into_owned();
    for name in HevcEncoder::PREFERENCE {
        let listed = listing
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name));
        if !listed {
            continue;
        }
        let encoder = HevcEncoder { name };
        let mut args = os_args(["-hide_banner", "-v", "error"]);
        args.extend(os_args(encoder.input_args()));
        args.extend(os_args([
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=256x256:r=30:d=0.2",
            "-vf",
            encoder.upload_filter(),
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ]));
        if crate::tools::run_capture(&tools.ffmpeg, args, &CancellationToken::new())
            .await
            .is_ok()
        {
            return Some(encoder);
        }
    }
    None
}
