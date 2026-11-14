//! Smooth Motion (Swift's `SmoothMotionRenderer`): the clip with frames made between its own, except across a hard
//! cut, where the nearer of the two frames is repeated rather than blending two shots into a ghost.
//!
//! ffmpeg's `minterpolate` makes the frames between. Its own scene detection (`scd=fdiff`) is not the macOS rule: it
//! compares whole frames and fires only when the difference also jumps against the pair before, so a cut after fast
//! motion can blend. The macOS rule is kept instead: the source's frames are read alongside, each pair is compared
//! by `CutDetector` on 360-pixel grey thumbnails, and the frames of `FrameTimeline` are chosen here exactly as the
//! macOS loop chooses them; `minterpolate` (with `scd=none`) only supplies the frames made between a pair that isn't
//! a cut. Three ffmpeg processes run side by side: the source's frames, the interpolated frames on the result's own
//! timeline, and the encoder that takes the frames chosen.

use std::ffi::OsString;
use std::path::Path;

use image::{ImageBuffer, Rgb, imageops::FilterType};
use jutsu_core::{
    CutDetector, EnhanceCodec, FrameTimeline, OnDeviceOutput, PixelSize, Progress, SmoothMotion,
};
use tokio_util::sync::CancellationToken;

use super::raw::{Encode, FrameReader, FrameWriter, Sound};
use crate::error::MediaError;
use crate::probe::probe_streams;
use crate::tools::{FfmpegTools, os_args};

/// `minterpolate`'s motion-compensated mode at its best quality settings: adaptive overlapped block motion
/// compensation, bidirectional motion estimation and variable-size blocks.
const INTERPOLATION: &str = "mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1:scd=none";

/// Makes `motion`'s result from the clip at `source`, written to `output` as an MP4. `progress` is 0…1 of the
/// result's frames. Cancelling kills ffmpeg; failing or cancelling leaves nothing at `output`.
pub async fn render(
    tools: &FfmpegTools,
    motion: SmoothMotion,
    source: &Path,
    output: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let result = render_frames(tools, motion, source, output, progress, cancel).await;
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

async fn render_frames(
    tools: &FfmpegTools,
    motion: SmoothMotion,
    source: &Path,
    output: &Path,
    progress: Option<Progress<f64>>,
    cancel: &CancellationToken,
) -> Result<OnDeviceOutput, MediaError> {
    let report = probe_streams(tools, source.as_os_str()).await?;
    let video = report.playable_video()?.clone();
    let size = video.display;
    if !size.is_valid() {
        return Err(MediaError::UnsupportedFormat);
    }
    let duration = if video.time.duration > 0.0 {
        video.time.duration
    } else {
        report.duration
    };
    // note: a source frame's time is its index over the nominal rate, which holds for the constant-rate clips the
    // generators make; a variable-rate clip gets its frames placed by that rate.
    let source_rate = if video.frame_rate >= 1.0 {
        video.frame_rate
    } else {
        30.0
    };
    let rate = motion.output_frame_rate(video.frame_rate);
    let mut timeline = FrameTimeline::new(duration, rate, motion.slowdown());
    let count = timeline.frame_count();

    let mut frames = FrameReader::spawn(tools, source_args(source), size)?;
    let mut between = FrameReader::spawn(
        tools,
        interpolation_args(source, motion, source_rate, rate, count),
        size,
    )?;
    let sound = if motion.keeps_audio() {
        Sound::FromSource
    } else {
        Sound::Without
    };
    let encode = Encode {
        size,
        output_size: size,
        frame_rate: rate as f64,
        codec: EnhanceCodec::of(size, video.codec == "avc1"),
    };
    let mut writer = FrameWriter::spawn(tools, encode, source, sound, output).await?;

    let Some(mut earlier) = frames.next(cancel).await? else {
        return Err(MediaError::UnsupportedFormat);
    };
    let mut earlier_index: i64 = 0;
    let mut earlier_luma = luma_thumbnail(&earlier, size);
    while timeline.next() < count {
        let later = frames.next(cancel).await?;
        let later_luma = later.as_ref().map(|frame| luma_thumbnail(frame, size));
        let is_cut = later_luma
            .as_ref()
            .is_some_and(|luma| CutDetector::is_cut(&earlier_luma, luma));
        let earlier_time = earlier_index as f64 / source_rate;
        let later_time = later.as_ref().map(|_| (earlier_index + 1) as f64 / source_rate);
        for frame in timeline.frames(earlier_time, later_time) {
            // The interpolated stream runs on the result's timeline: its frame n is output frame n.
            let made = between.next(cancel).await?;
            let chosen = choose(frame.phase, is_cut, later.as_deref(), made.as_deref(), &earlier);
            writer.write(chosen, cancel).await?;
            if let Some(progress) = &progress {
                progress((frame.index + 1) as f64 / count.max(1) as f64);
            }
        }
        let (Some(later), Some(later_luma)) = (later, later_luma) else {
            break;
        };
        earlier = later;
        earlier_luma = later_luma;
        earlier_index += 1;
    }
    writer.finish(cancel).await?;
    Ok(OnDeviceOutput {
        pixel_size: size,
        duration: timeline.output_time(count),
        file_extension: "mp4".to_owned(),
    })
}

/// The frame shown at `phase` between `earlier` and `later`: the earlier frame itself at phase 0 and after the last
/// one; across a cut the nearer of the two; otherwise the frame made between them, or the earlier one when none
/// came.
fn choose<'a>(
    phase: f64,
    is_cut: bool,
    later: Option<&'a [u8]>,
    made: Option<&'a [u8]>,
    earlier: &'a [u8],
) -> &'a [u8] {
    match later {
        Some(later) if is_cut && phase >= 0.5 => later,
        Some(_) if !is_cut && phase > 0.0 => made.unwrap_or(earlier),
        _ => earlier,
    }
}

/// The source's frames as they are, upright.
fn source_args(source: &Path) -> Vec<OsString> {
    let mut args = os_args(["-i"]);
    args.push(source.as_os_str().to_owned());
    args.extend(os_args(["-map", "0:v:0", "-an", "-fps_mode", "passthrough"]));
    args
}

/// The frames `minterpolate` makes on the result's timeline: the clip slowed down `slowdown` times, at `rate` frames a
/// second. The last frame is held for two more source frames so the filter, which looks ahead, hands out every frame
/// up to the end; `count` frames are taken.
fn interpolation_args(
    source: &Path,
    motion: SmoothMotion,
    source_rate: f64,
    rate: i64,
    count: i64,
) -> Vec<OsString> {
    let slowdown = motion.slowdown();
    let hold = 2.0 * slowdown as f64 / source_rate;
    let filter = format!(
        "setpts={slowdown}*(PTS-STARTPTS),tpad=stop_mode=clone:stop_duration={hold},minterpolate=fps={rate}:{INTERPOLATION}"
    );
    let mut args = os_args(["-i"]);
    args.push(source.as_os_str().to_owned());
    args.extend(os_args([
        "-map".to_owned(),
        "0:v:0".to_owned(),
        "-an".to_owned(),
        "-vf".to_owned(),
        filter,
        "-fps_mode".to_owned(),
        "passthrough".to_owned(),
        "-frames:v".to_owned(),
        count.to_string(),
    ]));
    args
}

/// A grey thumbnail of a packed RGB frame for `CutDetector` (Swift's `LumaSampler`): `CutDetector::THUMBNAIL_SIDE`
/// on the long side whatever the frame's size, scaled with a Lanczos filter so every source pixel counts, then each
/// pixel's luma as 0.299 R + 0.587 G + 0.114 B.
pub fn luma_thumbnail(frame: &[u8], size: PixelSize) -> Vec<u8> {
    let (Ok(width), Ok(height)) = (u32::try_from(size.width), u32::try_from(size.height)) else {
        return Vec::new();
    };
    let Some(image) = ImageBuffer::<Rgb<u8>, &[u8]>::from_raw(width, height, frame) else {
        return Vec::new();
    };
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let scale = CutDetector::THUMBNAIL_SIDE as f64 / f64::from(width.max(height));
    let small_width = ((f64::from(width) * scale).round() as u32).max(1);
    let small_height = ((f64::from(height) * scale).round() as u32).max(1);
    let small = image::imageops::resize(&image, small_width, small_height, FilterType::Lanczos3);
    small
        .pixels()
        .map(|pixel| {
            let [red, green, blue] = pixel.0;
            let luma = 0.299 * f64::from(red) + 0.587 * f64::from(green) + 0.114 * f64::from(blue);
            luma.round().min(255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::test_support::{Video, adding_tone, dir, shade, tools};

    #[test]
    fn a_cut_holds_the_nearer_frame_and_motion_takes_the_made_one() {
        let earlier = [1_u8];
        let later = [2_u8];
        let made = [3_u8];
        assert_eq!(choose(0.0, false, Some(&later), Some(&made), &earlier), earlier);
        assert_eq!(choose(0.5, false, Some(&later), Some(&made), &earlier), made);
        assert_eq!(choose(0.5, false, Some(&later), None, &earlier), earlier);
        assert_eq!(choose(0.49, true, Some(&later), Some(&made), &earlier), earlier);
        // Swift's rule: from half way on, the later frame.
        assert_eq!(choose(0.5, true, Some(&later), Some(&made), &earlier), later);
        assert_eq!(choose(0.0, false, None, Some(&made), &earlier), earlier);
    }

    #[test]
    fn thumbnails_are_360_on_the_long_side_and_tell_a_cut_from_a_fade() {
        let size = PixelSize::new(32, 18);
        let frame = |level: u8| vec![level; 32 * 18 * 3];
        let dark = luma_thumbnail(&frame(40), size);
        assert_eq!(dark.len(), 360 * 203);
        assert!(dark.iter().all(|value| *value == 40));
        assert!(!CutDetector::is_cut(&dark, &luma_thumbnail(&frame(60), size)));
        assert!(CutDetector::is_cut(&dark, &luma_thumbnail(&frame(200), size)));
    }

    /// 10 fps, 1 s, 160×90: a slow fade (10 levels a frame, no cut) for five frames, then a hard cut to a bright shot.
    fn clip_with_a_cut() -> Video {
        Video::new(1)
            .size(160, 90)
            .rate(10)
            .shade(|index| if index < 5 { 40 + index as u8 * 10 } else { 220 })
    }

    fn near(value: u8, expected: u8) -> bool {
        value.abs_diff(expected) <= 4
    }

    #[tokio::test]
    async fn sixty_fps_makes_frames_between_but_none_across_the_cut_and_keeps_the_sound() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let silent = clip_with_a_cut().write(&tools, folder.path()).await;
        let source = adding_tone(&tools, &silent, folder.path()).await;
        let output = folder.path().join("result.mp4");
        let fractions = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&fractions);
        let progress: Progress<f64> = Arc::new(move |value| log.lock().unwrap().push(value));

        let result = render(
            &tools,
            SmoothMotion::Fps60,
            &source,
            &output,
            Some(progress),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(result.pixel_size, PixelSize::new(160, 90));
        assert!((result.duration - 1.0).abs() < 1e-9);
        let info = crate::probe::probe(&tools, &output).await.unwrap();
        assert!((info.frame_rate - 60.0).abs() < 0.01, "{}", info.frame_rate);
        assert!(info.has_audio);
        assert_eq!(fractions.lock().unwrap().last().copied(), Some(1.0));
        // Source frame 4 (80) is at 24/60 s, frame 5 (220) at 30/60 s.
        let at = |frame: i64| frame as f64 / 60.0 + 0.001;
        assert!(near(shade(&tools, &output, at(24)).await, 80));
        assert!(near(shade(&tools, &output, at(26)).await, 80));
        // Phase 0.5 and later across the cut: the later shot, never a blend of the two.
        assert!(near(shade(&tools, &output, at(27)).await, 220));
        assert!(near(shade(&tools, &output, at(29)).await, 220));
        // Within the fade, half way from 40 to 50: a frame made between them.
        let made = shade(&tools, &output, at(3)).await;
        assert!((42..=48).contains(&made), "{made}");
    }

    #[tokio::test]
    async fn slow_motion_lasts_longer_at_the_clips_rate_without_sound() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let silent = clip_with_a_cut().write(&tools, folder.path()).await;
        let source = adding_tone(&tools, &silent, folder.path()).await;
        let output = folder.path().join("slow.mp4");

        let result = render(
            &tools,
            SmoothMotion::SlowMotion2x,
            &source,
            &output,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!((result.duration - 2.0).abs() < 1e-9);
        let info = crate::probe::probe(&tools, &output).await.unwrap();
        assert!((info.frame_rate - 10.0).abs() < 0.01);
        assert!((info.duration - 2.0).abs() < 0.15, "{}", info.duration);
        assert!(!info.has_audio);
    }

    #[tokio::test]
    async fn cancelling_leaves_nothing() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = clip_with_a_cut().write(&tools, folder.path()).await;
        let output = folder.path().join("cancelled.mp4");
        let cancel = CancellationToken::new();
        cancel.cancel();

        let outcome = render(&tools, SmoothMotion::Fps60, &source, &output, None, &cancel).await;

        assert_eq!(outcome, Err(MediaError::Cancelled));
        assert!(!output.exists());
    }
}
