//! "Join with original" (Swift's `ClipJoiner`): a source clip and the part Seedance's Extend added after it, as one
//! file. The API returns only the new part, so the two are put together here.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

use jutsu_core::{ClosedRange, PixelSize};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::editing::move_no_replace;
use crate::error::MediaError;
use crate::probe::{StreamTime, probe_streams};
use crate::tools::{EncodeQuality, FfmpegRun, FfmpegTools, os_args, run_capture};
use crate::video::AUDIO_BIT_RATE;

/// One part of the joined file: which input, the stretch of its picture, and the stretch of its sound inside it.
struct Part {
    start: f64,
    end: f64,
    sound: Option<(f64, f64)>,
    rate: f64,
}

impl Part {
    fn length(&self) -> f64 {
        self.end - self.start
    }
}

/// Writes `range` of `original` followed by `addition` to `output` as H.264 MP4, at the original's frame size; the
/// extension is scaled to fill it. The extension's first frame is the original's last, so it is left out, and the
/// extension's sound with it, to keep that sound in step with its picture. A part without sound is silent in the
/// joined file. A failed or cancelled join leaves nothing at `output`, and an existing file there is never replaced.
pub async fn join(
    tools: &FfmpegTools,
    original: &Path,
    range: ClosedRange<f64>,
    addition: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    if output.exists() {
        return Err(MediaError::FileExists(output.to_path_buf()));
    }
    let mut parts = Vec::with_capacity(2);
    let mut render_size: Option<PixelSize> = None;
    for (path, selection) in [(original, Some(range)), (addition, None)] {
        let report = probe_streams(tools, path.as_os_str()).await?;
        let video = report.video.as_ref().ok_or(MediaError::UnsupportedFormat)?;
        let whole = video.time;
        let (start, end) = match selection {
            Some(range) => (range.lower().max(whole.start), range.upper().min(whole.end())),
            None => dropping_first_frame(tools, path, whole, video.frame_rate).await,
        };
        if end <= start {
            return Err(MediaError::InvalidRange);
        }
        let sound = report.audio.and_then(|audio| {
            let (from, to) = (audio.start.max(start), audio.end().min(end));
            (to > from).then_some((from, to))
        });
        render_size.get_or_insert_with(|| even_size(video.display));
        parts.push(Part {
            start,
            end,
            sound,
            rate: video.frame_rate,
        });
    }
    let render_size = render_size.ok_or(MediaError::UnsupportedFormat)?;
    let rate = parts.iter().map(|part| part.rate).fold(0.0, f64::max);
    let rate = if rate > 0.0 { rate.round() } else { 30.0 };
    let encoder = tools.h264_encoder().await?;

    let has_sound = parts.iter().any(|part| part.sound.is_some());
    let graph = filter_graph(&parts, render_size, rate, encoder.upload_filter());

    let folder = output.parent().unwrap_or_else(|| Path::new("."));
    let partial = folder.join(format!(
        ".{}.partial.mp4",
        jutsu_core::support::uuid_string(uuid::Uuid::new_v4())
    ));
    let mut args: Vec<OsString> = os_args(["-y"]);
    args.extend(os_args(encoder.input_args()));
    for path in [original, addition] {
        args.push("-i".into());
        args.push(path.as_os_str().to_owned());
    }
    args.push("-filter_complex".into());
    args.push(graph.into());
    args.extend(os_args(["-map", "[v]"]));
    if has_sound {
        args.extend(os_args([
            "-map".to_owned(),
            "[a]".to_owned(),
            "-c:a".to_owned(),
            "aac".to_owned(),
            "-b:a".to_owned(),
            AUDIO_BIT_RATE.to_string(),
        ]));
    }
    args.extend(os_args(encoder.codec_args(
        EncodeQuality::Highest,
        (render_size.width * render_size.height) as f64 * rate,
    )));
    args.extend(os_args(["-movflags", "+faststart", "-f", "mp4"]));
    args.push(partial.as_os_str().to_owned());
    let total = parts.iter().map(Part::length).sum();
    let run = FfmpegRun {
        args,
        output: Some(&partial),
        duration: total,
        progress: None,
        working_directory: None,
    }
    .run(tools, cancel)
    .await;
    let moved = run.and_then(|()| move_no_replace(&partial, output));
    let _ = std::fs::remove_file(&partial);
    moved
}

/// The pictures trimmed, scaled to `render_size` and joined at `rate`; with any sound, the sounds trimmed to their
/// stretches and padded with silence to their parts' lengths, joined in step.
fn filter_graph(parts: &[Part], render_size: PixelSize, rate: f64, upload_filter: &str) -> String {
    let mut graph = String::new();
    for (index, part) in parts.iter().enumerate() {
        let _ = write!(
            graph,
            "[{index}:v]trim=start={:.6}:end={:.6},setpts=PTS-STARTPTS,scale={}:{}:flags=bicubic,setsar=1[v{index}];",
            part.start, part.end, render_size.width, render_size.height
        );
    }
    let _ = write!(graph, "[v0][v1]concat=n=2:v=1:a=0,fps={rate},{upload_filter}[v]");
    if parts.iter().any(|part| part.sound.is_some()) {
        for (index, part) in parts.iter().enumerate() {
            let length = part.length();
            match part.sound {
                Some((from, to)) => {
                    let delay = ((from - part.start) * 1000.0).round().max(0.0);
                    let _ = write!(
                        graph,
                        ";[{index}:a]atrim=start={from:.6}:end={to:.6},asetpts=PTS-STARTPTS,aresample=48000,\
                         aformat=sample_fmts=fltp:channel_layouts=stereo,adelay={delay}:all=1,\
                         apad=whole_dur={length:.6},atrim=end={length:.6}[a{index}]"
                    );
                }
                None => {
                    let _ = write!(
                        graph,
                        ";anullsrc=r=48000:cl=stereo,atrim=end={length:.6},\
                         aformat=sample_fmts=fltp:channel_layouts=stereo[a{index}]"
                    );
                }
            }
        }
        graph.push_str(";[a0][a1]concat=n=2:v=0:a=1[a]");
    }
    graph
}

/// The stretch without its first frame, which an extension shares with the clip it continues. Its own frame length,
/// so an extension of another frame rate than the original loses exactly one frame. A part of a single frame stays
/// whole.
async fn dropping_first_frame(tools: &FfmpegTools, path: &Path, whole: StreamTime, rate: f64) -> (f64, f64) {
    let frame = first_frame_duration(tools, path)
        .await
        .unwrap_or(if rate > 0.0 { 1.0 / rate } else { 0.0 });
    if frame > 0.0 && frame < whole.duration {
        (whole.start + frame, whole.end())
    } else {
        (whole.start, whole.end())
    }
}

/// From the first frame's presentation time to the second's.
async fn first_frame_duration(tools: &FfmpegTools, path: &Path) -> Option<f64> {
    #[derive(Deserialize)]
    struct Frames {
        #[serde(default)]
        frames: Vec<Frame>,
    }
    #[derive(Deserialize)]
    struct Frame {
        best_effort_timestamp_time: Option<String>,
        pts_time: Option<String>,
    }
    let mut args = os_args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-read_intervals",
        "%+#8",
        "-show_entries",
        "frame=best_effort_timestamp_time,pts_time",
        "-of",
        "json",
    ]);
    args.push(path.as_os_str().to_owned());
    let output = run_capture(&tools.ffprobe, args, &CancellationToken::new())
        .await
        .ok()?;
    let frames: Frames = serde_json::from_slice(&output).ok()?;
    let mut times: Vec<f64> = frames
        .frames
        .iter()
        .filter_map(|frame| {
            frame
                .best_effort_timestamp_time
                .as_deref()
                .or(frame.pts_time.as_deref())?
                .parse()
                .ok()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    let frame = times.get(1)? - times.first()?;
    (frame > 0.0).then_some(frame)
}

/// H.264 needs even sides.
fn even_size(size: PixelSize) -> PixelSize {
    let even = |side: i64| (((side.abs() as f64) / 2.0).round() as i64 * 2).max(2);
    PixelSize::new(even(size.width), even(size.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::probe;
    use crate::test_support::{Video, adding_tone, dir, shade, tools};

    /// The selection of the original and the extension end to end: as long as both together less the extension's
    /// first frame (0.2 s at 5 fps), so there is no gap at the seam, at the original's frame size although the
    /// extension is smaller.
    #[tokio::test]
    async fn joins_the_selection_and_the_extension_without_a_gap() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let original = Video::new(3).write(&tools, folder.path()).await;
        let addition = Video::new(2).size(360, 240).write(&tools, folder.path()).await;
        let output = folder.path().join("joined.mp4");

        join(
            &tools,
            &original,
            ClosedRange::new(1.0, 3.0),
            &addition,
            &output,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let info = probe(&tools, &output).await.unwrap();

        assert!((info.duration - 3.8).abs() < 0.05, "{}", info.duration);
        assert_eq!(info.pixel_size, PixelSize::new(720, 480));
        assert_eq!(info.codec, "avc1");
        assert!(
            !std::fs::read_dir(folder.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".partial"))
        );
    }

    /// An Extend result starts on the original's last frame. The joined file shows it once, the extension's next
    /// frame follows, and each part's sound plays under its own picture, although the parts run at different frame
    /// rates.
    #[tokio::test]
    async fn the_seam_frame_is_shown_once_and_the_sound_stays_in_step() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        // 10 fps, grey 20, 40, … 200; the last frame is 200.
        let original_picture = Video::new(1)
            .size(320, 240)
            .rate(10)
            .shade(|index| (20 + 20 * index) as u8)
            .write(&tools, folder.path())
            .await;
        // 5 fps, starting on the original's last frame: 200, then 60, 100, 140, 180.
        let addition_picture = Video::new(1)
            .size(320, 240)
            .rate(5)
            .shade(|index| if index == 0 { 200 } else { (20 + 40 * index) as u8 })
            .write(&tools, folder.path())
            .await;
        let original = adding_tone(&tools, &original_picture, folder.path()).await;
        let addition = adding_tone(&tools, &addition_picture, folder.path()).await;
        let output = folder.path().join("joined.mp4");

        join(
            &tools,
            &original,
            ClosedRange::new(0.0, 1.0),
            &addition,
            &output,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        let report = probe_streams(&tools, output.as_os_str()).await.unwrap();
        let picture = report.video.unwrap().time;
        let sound = report.audio.unwrap();
        // One second of each, less the extension's first frame of 0.2 s; a repeated seam frame would add 0.2 s.
        assert!((picture.duration - 1.8).abs() < 0.11, "{}", picture.duration);
        assert!((sound.start - picture.start).abs() < 0.1);
        assert!(
            (sound.end() - picture.end()).abs() < 0.1,
            "{} {}",
            sound.end(),
            picture.end()
        );

        assert!((i32::from(shade(&tools, &output, 0.95).await) - 200).abs() < 16);
        assert!((i32::from(shade(&tools, &output, 1.05).await) - 60).abs() < 16);
        assert!((i32::from(shade(&tools, &output, 1.25).await) - 100).abs() < 16);
    }

    #[tokio::test]
    async fn never_replaces_an_existing_file() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let original = Video::new(1).size(320, 240).write(&tools, folder.path()).await;
        let addition = Video::new(1).size(320, 240).write(&tools, folder.path()).await;
        let output = folder.path().join("joined.mp4");
        std::fs::write(&output, "keep me").unwrap();

        let outcome = join(
            &tools,
            &original,
            ClosedRange::new(0.0, 1.0),
            &addition,
            &output,
            &CancellationToken::new(),
        )
        .await;
        assert!(outcome.is_err());
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "keep me");
    }
}
