//! The ffmpeg and ffprobe executables, what they can do, and running them.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};

use jutsu_core::Progress;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;

/// The ffmpeg and ffprobe executables every function runs. The app passes its bundled sidecars.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FfmpegTools {
    /// The `ffmpeg` executable.
    pub ffmpeg: PathBuf,
    /// The `ffprobe` executable.
    pub ffprobe: PathBuf,
}

impl FfmpegTools {
    /// The tools at these paths.
    pub fn new(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    /// The tools the `FFMPEG` and `FFPROBE` environment variables name, else the copies installed next to the app's
    /// executable (Tauri's `externalBin` sidecars), else the first `ffmpeg` and `ffprobe` on `PATH`; `None` when
    /// either can't be found.
    pub fn from_environment() -> Option<Self> {
        let find = |variable: &str, name: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| beside_executable(name))
                .or_else(|| find_on_path(name))
        };
        Some(Self::new(find("FFMPEG", "ffmpeg")?, find("FFPROBE", "ffprobe")?))
    }
}

/// The copy the installer put next to the app's own executable.
fn beside_executable(name: &str) -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    Some(folder.join(file)).filter(|path| path.is_file())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_owned()]
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&path)
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| candidate.is_file())
}

/// An H.264 encoder ffmpeg offers. The bundled build is LGPL, so it has no `libx264`: the encoder is chosen at run
/// time from what the build lists and what works on this computer (`FfmpegTools::h264_encoder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum H264Encoder {
    /// x264 (GPL builds only).
    Libx264,
    /// NVIDIA NVENC.
    Nvenc,
    /// Intel Quick Sync.
    Qsv,
    /// AMD AMF.
    Amf,
    /// VA-API (Linux).
    Vaapi,
    /// Apple VideoToolbox.
    VideoToolbox,
    /// Windows Media Foundation.
    MediaFoundation,
    /// Cisco's OpenH264.
    OpenH264,
}

/// How hard an encode tries: the quality of an export preset, or a bitrate to fit a size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EncodeQuality {
    /// AVFoundation's `HighestQuality` preset.
    Highest,
    /// AVFoundation's `MediumQuality` preset.
    Medium,
    /// An average bitrate in bits per second.
    BitRate(i64),
}

impl H264Encoder {
    /// In the order they are preferred.
    pub const PREFERENCE: [Self; 8] = [
        Self::Libx264,
        Self::Nvenc,
        Self::Qsv,
        Self::Amf,
        Self::Vaapi,
        Self::VideoToolbox,
        Self::MediaFoundation,
        Self::OpenH264,
    ];

    /// ffmpeg's name for it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Libx264 => "libx264",
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::Vaapi => "h264_vaapi",
            Self::VideoToolbox => "h264_videotoolbox",
            Self::MediaFoundation => "h264_mf",
            Self::OpenH264 => "libopenh264",
        }
    }

    /// Options that go before the inputs: VA-API needs its device.
    pub(crate) fn input_args(self) -> Vec<String> {
        match self {
            Self::Vaapi => vec!["-vaapi_device".into(), "/dev/dri/renderD128".into()],
            _ => Vec::new(),
        }
    }

    /// The filter that hands frames to the encoder in a format it takes; ends every video filter chain.
    pub(crate) fn upload_filter(self) -> &'static str {
        match self {
            Self::Vaapi => "format=nv12,hwupload",
            Self::Qsv => "format=nv12",
            _ => "format=yuv420p",
        }
    }

    /// The encoder and its rate control for `quality`; `pixels_per_second` (width × height × rate) sets the bitrate
    /// of an encoder without a constant-quality mode.
    pub(crate) fn codec_args(self, quality: EncodeQuality, pixels_per_second: f64) -> Vec<String> {
        let mut args = vec!["-c:v".to_owned(), self.name().to_owned()];
        let bits_per_pixel = match quality {
            EncodeQuality::Highest => 0.25,
            EncodeQuality::Medium => 0.1,
            EncodeQuality::BitRate(_) => 0.0,
        };
        let bit_rate = match quality {
            EncodeQuality::BitRate(rate) => rate,
            _ => ((pixels_per_second * bits_per_pixel) as i64).max(500_000),
        };
        match (self, quality) {
            (Self::Libx264, EncodeQuality::Highest) => {
                args.extend(["-preset", "medium", "-crf", "18"].map(String::from));
            }
            (Self::Libx264, EncodeQuality::Medium) => {
                args.extend(["-preset", "medium", "-crf", "23"].map(String::from));
            }
            _ => {
                args.extend(["-b:v".to_owned(), bit_rate.to_string()]);
                if let EncodeQuality::BitRate(rate) = quality {
                    // A cap keeps encoders that overshoot an average inside the size budget.
                    args.extend([
                        "-maxrate".to_owned(),
                        rate.to_string(),
                        "-bufsize".to_owned(),
                        (rate * 2).to_string(),
                    ]);
                }
            }
        }
        if self == Self::Libx264 {
            args.extend(["-profile:v", "high"].map(String::from));
        }
        args
    }
}

/// What a probe of one ffmpeg found, kept for the process's life.
#[derive(Debug, Clone, Default)]
struct Abilities {
    encoder: Option<H264Encoder>,
    filters: Option<(bool, bool)>,
}

static ABILITIES: LazyLock<Mutex<HashMap<PathBuf, Abilities>>> = LazyLock::new(Mutex::default);

fn cached<T>(tools: &FfmpegTools, read: impl FnOnce(&Abilities) -> Option<T>) -> Option<T> {
    ABILITIES
        .lock()
        .ok()
        .and_then(|map| map.get(&tools.ffmpeg).and_then(read))
}

fn remember(tools: &FfmpegTools, write: impl FnOnce(&mut Abilities)) {
    if let Ok(mut map) = ABILITIES.lock() {
        write(map.entry(tools.ffmpeg.clone()).or_default());
    }
}

impl FfmpegTools {
    /// The H.264 encoder this ffmpeg encodes with: the first of `H264Encoder::PREFERENCE` that `ffmpeg -encoders`
    /// lists and that encodes a short test clip on this computer. Chosen once per ffmpeg.
    pub async fn h264_encoder(&self) -> Result<H264Encoder, MediaError> {
        if let Some(encoder) = cached(self, |abilities| abilities.encoder) {
            return Ok(encoder);
        }
        let listed = self.list("-encoders").await?;
        for encoder in H264Encoder::PREFERENCE {
            if !lists(&listed, encoder.name()) {
                continue;
            }
            if self.encodes_with(encoder).await {
                tracing::info!(encoder = encoder.name(), "H.264 encoder chosen");
                remember(self, |abilities| abilities.encoder = Some(encoder));
                return Ok(encoder);
            }
        }
        Err(MediaError::NoH264Encoder)
    }

    /// Whether this ffmpeg has libass's `subtitles` filter and FreeType's `drawtext` filter.
    pub async fn text_filters(&self) -> Result<(bool, bool), MediaError> {
        if let Some(filters) = cached(self, |abilities| abilities.filters) {
            return Ok(filters);
        }
        let listed = self.list("-filters").await?;
        let filters = (lists(&listed, "subtitles"), lists(&listed, "drawtext"));
        remember(self, |abilities| abilities.filters = Some(filters));
        Ok(filters)
    }

    async fn list(&self, what: &str) -> Result<String, MediaError> {
        let output = run_capture(
            &self.ffmpeg,
            ["-hide_banner", what].map(OsString::from),
            &CancellationToken::new(),
        )
        .await?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    async fn encodes_with(&self, encoder: H264Encoder) -> bool {
        let mut args: Vec<OsString> = ["-hide_banner", "-v", "error"].map(OsString::from).to_vec();
        args.extend(encoder.input_args().into_iter().map(OsString::from));
        args.extend(
            [
                "-f",
                "lavfi",
                "-i",
                "color=c=gray:s=256x256:r=30:d=0.2",
                "-vf",
                encoder.upload_filter(),
            ]
            .map(OsString::from),
        );
        args.extend(
            encoder
                .codec_args(EncodeQuality::Medium, 256.0 * 256.0 * 30.0)
                .into_iter()
                .map(OsString::from),
        );
        args.extend(["-f", "null", "-"].map(OsString::from));
        run_capture(&self.ffmpeg, args, &CancellationToken::new())
            .await
            .is_ok()
    }
}

/// Whether ffmpeg's `-encoders` or `-filters` listing names `name` (its second column).
fn lists(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name))
}

/// A command that never opens a console window on Windows and dies with its handle.
pub(crate) fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.kill_on_drop(true).stdin(Stdio::null());
    #[cfg(windows)]
    {
        /// `CREATE_NO_WINDOW`.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// How many lines of stderr an error keeps.
const STDERR_LINES: usize = 12;

/// Runs `program` with `args` and returns its stdout. A failure carries the last lines of its stderr.
pub(crate) async fn run_capture(
    program: &Path,
    args: impl IntoIterator<Item = OsString>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, MediaError> {
    let mut command = command(program);
    command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| MediaError::ExportFailed(format!("{}: {error}", program.display())))?;
    let mut stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read_out = async {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.read_to_end(&mut bytes).await;
        }
        bytes
    };
    let work = async { tokio::join!(read_out, tail(stderr), child.wait()) };
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(MediaError::Cancelled),
        (bytes, errors, status) = work => {
            let status = status?;
            if status.success() {
                Ok(bytes)
            } else {
                Err(MediaError::ExportFailed(failure(&errors, status)))
            }
        }
    }
}

/// One ffmpeg run that writes a file.
pub(crate) struct FfmpegRun<'a> {
    /// The arguments after the global ones this adds.
    pub args: Vec<OsString>,
    /// The file it writes, removed when the run fails or is cancelled.
    pub output: Option<&'a Path>,
    /// Seconds of output the run makes, which turn its position into a fraction.
    pub duration: f64,
    /// Where the fraction goes.
    pub progress: Option<Progress<f64>>,
    /// The folder it runs in, so a filter can name a file there without escaping its path.
    pub working_directory: Option<&'a Path>,
}

impl FfmpegRun<'_> {
    /// Runs it. Progress comes from `-progress pipe:1`; cancelling kills ffmpeg and removes the output.
    pub(crate) async fn run(self, tools: &FfmpegTools, cancel: &CancellationToken) -> Result<(), MediaError> {
        let result = self.run_inner(tools, cancel).await;
        if result.is_err()
            && let Some(output) = self.output
        {
            let _ = std::fs::remove_file(output);
        }
        result
    }

    async fn run_inner(&self, tools: &FfmpegTools, cancel: &CancellationToken) -> Result<(), MediaError> {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled);
        }
        let mut command = command(&tools.ffmpeg);
        command
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-nostats",
                "-progress",
                "pipe:1",
            ])
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(folder) = self.working_directory {
            command.current_dir(folder);
        }
        let mut child = command
            .spawn()
            .map_err(|error| MediaError::ExportFailed(format!("ffmpeg: {error}")))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let duration = self.duration;
        let progress = self.progress.clone();
        let report = async move {
            let Some(stdout) = stdout else { return };
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let micros = line
                    .strip_prefix("out_time_us=")
                    .or_else(|| line.strip_prefix("out_time_ms="))
                    .and_then(|value| value.trim().parse::<f64>().ok());
                if let (Some(micros), Some(progress)) = (micros, progress.as_ref())
                    && duration > 0.0
                {
                    progress((micros / 1_000_000.0 / duration).clamp(0.0, 1.0));
                }
            }
        };
        let work = async { tokio::join!(report, tail(stderr), child.wait()) };
        let ((), errors, status) = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(MediaError::Cancelled),
            done = work => done,
        };
        let status = status?;
        if !status.success() {
            return Err(MediaError::ExportFailed(failure(&errors, status)));
        }
        if let Some(progress) = &self.progress {
            progress(1.0);
        }
        Ok(())
    }
}

/// The last lines of a child's stderr.
pub(crate) async fn tail(stderr: Option<tokio::process::ChildStderr>) -> VecDeque<String> {
    let mut kept = VecDeque::new();
    let Some(stderr) = stderr else { return kept };
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if kept.len() == STDERR_LINES {
            kept.pop_front();
        }
        kept.push_back(line);
    }
    kept
}

pub(crate) fn failure(errors: &VecDeque<String>, status: std::process::ExitStatus) -> String {
    if errors.is_empty() {
        format!("ffmpeg ended with {status}")
    } else {
        errors.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// The arguments as `OsString`s.
pub(crate) fn os_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter().map(Into::into).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_names_its_second_column() {
        let listing = " V....D libx264              libx264 H.264\n V....D h264_videotoolbox    VideoToolbox";
        assert!(lists(listing, "libx264"));
        assert!(lists(listing, "h264_videotoolbox"));
        assert!(!lists(listing, "h264_nvenc"));
    }
}
