//! Re-encodes a downloaded result for an editing timeline, as an `ExportPreset` says (Swift's `EditingExporter`), with
//! an optional crop and subtitles drawn into the frames (`SubtitleBurnIn`).

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use jutsu_core::{
    CropAspect, CropFrame, ExportPreset, FrameDuration, PixelSize, Progress, SourceClip, SubtitleCue,
    VideoCodec,
};
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;
use crate::probe::{probe, probe_streams};
use crate::tools::{EncodeQuality, FfmpegRun, FfmpegTools, os_args};
use crate::video::AUDIO_BIT_RATE;

/// Writes `result` to `output`, which must not exist yet; an existing file is never replaced. The constant frame rate
/// is the source clip's when its file can be read, otherwise the result's own. A cancelled or failed export leaves
/// nothing at `output`.
/// - `crop`: the frame of a delivery output that crops, as `CropFrame::rect` places it on the upright picture; `None`
///   keeps the whole frame.
/// - `burn_in`: cues drawn into the frames, timed from the start of the result as the output is. ffmpeg draws them
///   with libass (`subtitles`) or FreeType (`drawtext`); a build with neither fails with `BurnInUnavailable` before
///   anything is written.
#[allow(
    clippy::too_many_arguments,
    reason = "the Swift function's parameters, plus the tools and the token"
)]
pub async fn export(
    tools: &FfmpegTools,
    result: &Path,
    source: Option<&SourceClip>,
    preset: ExportPreset,
    crop: Option<(CropAspect, CropFrame)>,
    burn_in: &[SubtitleCue],
    output: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    if output.exists() {
        return Err(MediaError::FileExists(output.to_path_buf()));
    }
    let report = probe_streams(tools, result.as_os_str()).await?;
    let video = report.playable_video()?.clone();
    let source_rate = source_frame_rate(tools, source).await;
    let frame = FrameDuration::constant(source_rate.unwrap_or(video.frame_rate));

    let folder = output.parent().unwrap_or_else(|| Path::new("."));
    let mut scratch = Scratch(Vec::new());
    let mut filters = Vec::new();
    let mut size = video.display;
    if let Some((aspect, crop_frame)) = crop {
        let rect = crop_frame.rect(aspect, video.display);
        // Sides are rounded down to even numbers, which the encoders need.
        let width = (rect.width - rect.width % 2).max(2);
        let height = (rect.height - rect.height % 2).max(2);
        filters.push(format!("crop={width}:{height}:{}:{}", rect.x, rect.y));
        size = PixelSize::new(width, height);
    } else if preset.video_codec() == VideoCodec::H264 && (size.width % 2 != 0 || size.height % 2 != 0) {
        size = crate::video::even_floor(size);
        filters.push(format!("crop={}:{}:0:0", size.width, size.height));
    }
    if !burn_in.is_empty() {
        filters.push(burn_in_filter(tools, burn_in, size, folder, &mut scratch).await?);
    }

    let mut args: Vec<OsString> = os_args(["-y"]);
    let encoder = match preset.video_codec() {
        VideoCodec::H264 => Some(tools.h264_encoder().await?),
        VideoCodec::ProRes422 => None,
    };
    if let Some(encoder) = encoder {
        args.extend(os_args(encoder.input_args()));
    }
    args.push("-i".into());
    args.push(result.as_os_str().to_owned());
    args.extend(os_args(["-map", "0:v:0", "-map", "0:a?", "-sn", "-dn"]));
    match encoder {
        Some(encoder) => filters.push(encoder.upload_filter().to_owned()),
        None => filters.push("format=yuv422p10le".to_owned()),
    }
    args.push("-vf".into());
    args.push(filters.join(",").into());
    // Every frame at a fixed duration, which is what makes the output constant frame rate.
    args.extend(os_args([
        "-fps_mode".to_owned(),
        "cfr".to_owned(),
        "-r".to_owned(),
        format!("{}/{}", frame.timescale, frame.value),
    ]));
    match encoder {
        Some(encoder) => {
            let rate = f64::from(frame.timescale) / frame.value as f64;
            args.extend(os_args(encoder.codec_args(
                EncodeQuality::Highest,
                (size.width * size.height) as f64 * rate,
            )));
            args.extend(os_args([
                "-c:a".to_owned(),
                "aac".to_owned(),
                "-b:a".to_owned(),
                AUDIO_BIT_RATE.to_string(),
                "-movflags".to_owned(),
                "+faststart".to_owned(),
                "-f".to_owned(),
                "mp4".to_owned(),
            ]));
        }
        // ProRes 422 with PCM sound, as AVFoundation's `AppleProRes422LPCM` preset writes it.
        None => args.extend(os_args([
            "-c:v",
            "prores_ks",
            "-profile:v",
            "2",
            "-vendor",
            "apl0",
            "-c:a",
            "pcm_s16le",
            "-f",
            "mov",
        ])),
    }
    let partial = folder.join(format!(
        ".{}.partial.{}",
        jutsu_core::support::uuid_string(uuid::Uuid::new_v4()),
        preset.file_extension()
    ));
    args.push(partial.as_os_str().to_owned());
    scratch.0.push(partial.clone());
    FfmpegRun {
        args,
        output: Some(&partial),
        duration: report.duration,
        progress,
        working_directory: Some(folder),
    }
    .run(tools, cancel)
    .await?;
    move_no_replace(&partial, output)
}

/// Files an export writes beside its output and removes when it ends, however it ends.
struct Scratch(Vec<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Moves `from` to `to` unless a file is there: an existing file is never replaced.
pub(crate) fn move_no_replace(from: &Path, to: &Path) -> Result<(), MediaError> {
    match std::fs::hard_link(from, to) {
        Ok(()) => {
            let _ = std::fs::remove_file(from);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(MediaError::FileExists(to.to_path_buf()))
        }
        // A volume without hard links (FAT, exFAT): check, then rename.
        Err(_) if to.exists() => Err(MediaError::FileExists(to.to_path_buf())),
        Err(_) => std::fs::rename(from, to).map_err(MediaError::from),
    }
}

/// `None` when the source is gone, not readable or reports no rate.
async fn source_frame_rate(tools: &FfmpegTools, source: Option<&SourceClip>) -> Option<f64> {
    let path = source?.file_url.to_file_path().ok()?;
    let rate = probe(tools, &path).await.ok()?.frame_rate;
    (rate > 0.0).then_some(rate)
}

/// Above the bottom edge, as a share of the frame's height.
pub const BOTTOM_MARGIN: f64 = 0.08;
/// The text's width at most, as a share of the frame's width.
pub const WIDTH_SHARE: f64 = 0.9;

/// The point size for a frame: 4.5% of its height, smaller where two 42-character lines wouldn't fit its width.
pub fn font_size(size: PixelSize) -> f64 {
    (size.height as f64 * 0.045)
        .min(size.width as f64 * WIDTH_SHARE / 23.0)
        .round()
        .max(8.0)
}

/// The filter that draws `cues` on a frame of `size`: white semibold text with a soft shadow, centred inside the
/// bottom safe area. Its files go into `folder`, which ffmpeg runs in, so the filter names them without a path.
async fn burn_in_filter(
    tools: &FfmpegTools,
    cues: &[SubtitleCue],
    size: PixelSize,
    folder: &Path,
    scratch: &mut Scratch,
) -> Result<String, MediaError> {
    let (subtitles, drawtext) = tools.text_filters().await?;
    let stem = format!(".{}", jutsu_core::support::uuid_string(uuid::Uuid::new_v4()));
    if subtitles {
        let name = format!("{stem}.ass");
        let path = folder.join(&name);
        scratch.0.push(path.clone());
        std::fs::write(&path, ass_script(cues, size))?;
        return Ok(format!("subtitles=f={name}"));
    }
    if drawtext {
        let font = font_size(size);
        let margin = (size.height as f64 * BOTTOM_MARGIN).round();
        let mut parts = Vec::new();
        for (index, cue) in cues.iter().enumerate() {
            let lines = cue.lines();
            if cue.end <= cue.start || lines.is_empty() {
                continue;
            }
            let name = format!("{stem}-{index}.txt");
            let path = folder.join(&name);
            scratch.0.push(path.clone());
            std::fs::write(&path, lines.join("\n"))?;
            parts.push(format!(
                "drawtext=textfile={name}:font=Sans:fontsize={font}:fontcolor=white:line_spacing={}:\
                 shadowcolor=black@0.85:shadowx=0:shadowy=0:borderw={}:bordercolor=black@0.5:\
                 x=(w-text_w)/2:y=h-{margin}-text_h:enable='between(t,{:.3},{:.3})'",
                (font * 0.25).round(),
                (font * 0.06).round().max(1.0),
                cue.start,
                cue.end
            ));
        }
        return Ok(if parts.is_empty() {
            "null".to_owned()
        } else {
            parts.join(",")
        });
    }
    Err(MediaError::BurnInUnavailable)
}

/// The cues as an ASS script whose coordinates are the frame's pixels.
fn ass_script(cues: &[SubtitleCue], size: PixelSize) -> String {
    let font = font_size(size);
    let margin_v = (size.height as f64 * BOTTOM_MARGIN).round();
    let margin_h = (size.width as f64 * (1.0 - WIDTH_SHARE) / 2.0).round();
    let outline = (font * 0.06).round().max(1.0);
    let blur = (font * 0.25 / 2.0).round().max(1.0);
    let mut script = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: {}\nPlayResY: {}\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, \
         Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, \
         MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,sans-serif,{font},&H00FFFFFF,&H00FFFFFF,&H26000000,&H26000000,600,0,0,0,100,100,0,0,1,\
         {outline},0,2,{margin_h},{margin_h},{margin_v},1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        size.width, size.height
    );
    for cue in cues {
        let lines = cue.lines();
        if cue.end <= cue.start || lines.is_empty() {
            continue;
        }
        let text = lines
            .iter()
            .map(|line| line.replace('{', "(").replace('}', ")").replace('\\', "\u{2216}"))
            .collect::<Vec<_>>()
            .join("\\N");
        let _ = writeln!(
            script,
            "Dialogue: 0,{},{},Default,,0,0,0,,{{\\blur{blur}}}{text}",
            ass_time(cue.start),
            ass_time(cue.end)
        );
    }
    script
}

/// "0:01:02.35": hours, minutes, seconds and centiseconds.
fn ass_time(seconds: f64) -> String {
    let total = (seconds.max(0.0) * 100.0).round() as i64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        total / 360_000,
        total / 6000 % 60,
        total / 100 % 60,
        total % 100
    )
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;
    use crate::test_support::{Video, dir, tools};

    fn source(path: &Path) -> SourceClip {
        SourceClip::new(
            Url::from_file_path(path).unwrap(),
            2.0,
            PixelSize::new(720, 480),
            jutsu_core::ClosedRange::new(0.0, 2.0),
        )
    }

    fn leftovers(folder: &Path) -> bool {
        std::fs::read_dir(folder)
            .unwrap()
            .any(|entry| entry.unwrap().file_name().to_string_lossy().contains(".partial"))
    }

    #[tokio::test]
    async fn pro_res_takes_the_source_frame_rate() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let result = Video::new(2)
            .size(320, 240)
            .rate(30)
            .write(&tools, folder.path())
            .await;
        let original = Video::new(2)
            .size(320, 240)
            .rate(25)
            .write(&tools, folder.path())
            .await;
        let output = folder.path().join("take_v1.mov");

        export(
            &tools,
            &result,
            Some(&source(&original)),
            ExportPreset::FinalCutPro,
            None,
            &[],
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let info = probe(&tools, &output).await.unwrap();

        assert_eq!(info.codec, "apcn");
        assert!((info.frame_rate - 25.0).abs() < 0.5, "{}", info.frame_rate);
        assert!((info.duration - 2.0).abs() < 0.1, "{}", info.duration);
        assert!(!leftovers(folder.path()));
    }

    #[tokio::test]
    async fn h264_falls_back_to_the_result_frame_rate() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let result = Video::new(2)
            .size(320, 240)
            .rate(30)
            .write(&tools, folder.path())
            .await;
        let missing = folder.path().join("gone.mov");
        let output = folder.path().join("take_v1.mp4");

        export(
            &tools,
            &result,
            Some(&source(&missing)),
            ExportPreset::EditingH264,
            None,
            &[],
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let info = probe(&tools, &output).await.unwrap();

        assert_eq!(info.codec, "avc1");
        assert!((info.frame_rate - 30.0).abs() < 0.5, "{}", info.frame_rate);
    }

    #[tokio::test]
    async fn never_replaces_an_existing_file() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let result = Video::new(1).size(320, 240).write(&tools, folder.path()).await;
        let output = folder.path().join("take_v1.mov");
        std::fs::write(&output, "keep me").unwrap();

        let outcome = export(
            &tools,
            &result,
            None,
            ExportPreset::Premiere,
            None,
            &[],
            &output,
            None,
            &CancellationToken::new(),
        )
        .await;
        assert!(outcome.is_err());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "keep me");
    }

    #[test]
    fn text_is_sized_to_the_frame() {
        assert!((font_size(PixelSize::new(1920, 1080)) - 49.0).abs() < f64::EPSILON);
        assert!((font_size(PixelSize::new(1080, 1920)) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_ass_script_times_each_cue_and_skips_empty_ones() {
        let cues = [
            SubtitleCue::new(0.0, 1.0, "HELLO THERE"),
            SubtitleCue::new(1.2, 1.8, "Second\nline"),
            SubtitleCue::new(2.0, 3.0, "  "),
        ];
        let script = ass_script(&cues, PixelSize::new(640, 360));
        assert!(script.contains("PlayResX: 640\nPlayResY: 360"));
        assert!(script.contains("Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{\\blur2}HELLO THERE\n"));
        assert!(script.contains("0:00:01.20,0:00:01.80,Default,,0,0,0,,{\\blur2}Second\\Nline\n"));
        assert_eq!(script.matches("Dialogue:").count(), 2);
    }
}
