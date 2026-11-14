//! Sets a downloaded result's sound per `AudioPlan` (Swift's `AudioRestorer`). The video stream is copied, never
//! re-encoded.

use std::path::Path;

use jutsu_core::{AudioOption, AudioPlan, ClosedRange};
use tokio_util::sync::CancellationToken;

use crate::error::MediaError;
use crate::probe::probe_streams;
use crate::tools::{FfmpegRun, FfmpegTools, os_args};
use crate::video::AUDIO_BIT_RATE;

/// Writes `result` to `output` with its sound as `option` asks: the source range's audio cut to AAC (faded out when
/// asked) and put under the silent result, or the result without its sound. Returns `false` and writes nothing when
/// the result stays as it is. A failure leaves nothing at `output`, so the caller keeps the silent result.
pub async fn restore(
    tools: &FfmpegTools,
    option: AudioOption,
    range: ClosedRange<f64>,
    source: &Path,
    result: &Path,
    output: &Path,
) -> Result<bool, MediaError> {
    let report = probe_streams(tools, result.as_os_str()).await?;
    let video = report.video.as_ref().ok_or(MediaError::UnsupportedFormat)?;
    let video_duration = video.time.duration;
    let source_audio_end = if option == AudioOption::Mute {
        None
    } else {
        probe_streams(tools, source.as_os_str())
            .await
            .ok()
            .and_then(|source| source.audio)
            .map(crate::probe::StreamTime::end)
    };
    let plan = AudioPlan::make(
        option,
        range,
        source_audio_end,
        video_duration,
        report.audio.is_some(),
    );
    let _ = std::fs::remove_file(output);
    let outcome = match plan {
        AudioPlan::Keep => return Ok(false),
        AudioPlan::RemoveAudio => mux(tools, result, None, video_duration, output).await,
        AudioPlan::AddSource { range, fade_out } => {
            // Named `.<UUID>.m4a`: the engine's sweep deletes such scratch files after a crash.
            let folder = output.parent().unwrap_or_else(|| Path::new("."));
            let audio = folder.join(format!(
                ".{}.m4a",
                jutsu_core::support::uuid_string(uuid::Uuid::new_v4())
            ));
            let rendered = render_audio(tools, source, range, fade_out, &audio).await;
            let muxed = match rendered {
                Ok(()) => mux(tools, result, Some(&audio), video_duration, output).await,
                Err(error) => Err(error),
            };
            let _ = std::fs::remove_file(&audio);
            muxed
        }
    };
    match outcome {
        Ok(()) => Ok(true),
        Err(error) => {
            let _ = std::fs::remove_file(output);
            Err(error)
        }
    }
}

/// Cuts the source audio to AAC. It is re-encoded because a stream copy can't fade it, and because the source codec
/// (e.g. PCM from a .mov) may not fit in MP4.
async fn render_audio(
    tools: &FfmpegTools,
    source: &Path,
    range: ClosedRange<f64>,
    fade_out: Option<ClosedRange<f64>>,
    output: &Path,
) -> Result<(), MediaError> {
    let length = (range.upper() - range.lower()).max(0.0);
    let mut args = os_args([
        "-y".to_owned(),
        "-ss".to_owned(),
        format!("{:.6}", range.lower()),
        "-t".to_owned(),
        format!("{length:.6}"),
        "-i".to_owned(),
    ]);
    args.push(source.as_os_str().to_owned());
    args.extend(os_args(["-map", "0:a:0", "-vn", "-sn", "-dn"]));
    if let Some(fade) = fade_out {
        args.extend(os_args([
            "-af".to_owned(),
            format!(
                "afade=t=out:st={:.6}:d={:.6}",
                fade.lower(),
                (fade.upper() - fade.lower()).max(0.0)
            ),
        ]));
    }
    args.extend(os_args([
        "-c:a".to_owned(),
        "aac".to_owned(),
        "-b:a".to_owned(),
        AUDIO_BIT_RATE.to_string(),
        "-f".to_owned(),
        "ipod".to_owned(),
    ]));
    args.push(output.as_os_str().to_owned());
    FfmpegRun {
        args,
        output: Some(output),
        duration: length,
        progress: None,
        working_directory: None,
    }
    .run(tools, &CancellationToken::new())
    .await
}

/// The result's video with `audio`'s sound, cut to the video's length, or with no sound.
async fn mux(
    tools: &FfmpegTools,
    result: &Path,
    audio: Option<&Path>,
    video_duration: f64,
    output: &Path,
) -> Result<(), MediaError> {
    let mut args = os_args(["-y", "-i"]);
    args.push(result.as_os_str().to_owned());
    if let Some(audio) = audio {
        args.extend(os_args([
            "-t".to_owned(),
            format!("{video_duration:.6}"),
            "-i".to_owned(),
        ]));
        args.push(audio.as_os_str().to_owned());
    }
    args.extend(os_args(["-map", "0:v:0"]));
    if audio.is_some() {
        args.extend(os_args(["-map", "1:a:0"]));
    }
    args.extend(os_args(["-c", "copy", "-movflags", "+faststart", "-f", "mp4"]));
    args.push(output.as_os_str().to_owned());
    FfmpegRun {
        args,
        output: Some(output),
        duration: video_duration,
        progress: None,
        working_directory: None,
    }
    .run(tools, &CancellationToken::new())
    .await
}

#[cfg(test)]
mod tests {
    use jutsu_core::{JobSpec, Mode, PostAction, PostOperation, Resolution, SourceClip, VideoModel};
    use url::Url;

    use super::*;
    use crate::prepare::MediaPreparer;
    use crate::probe::probe;
    use crate::test_support::{Video, adding_tone, dir, tools};

    async fn clip(tools: &FfmpegTools, path: &Path, range: ClosedRange<f64>) -> SourceClip {
        let info = probe(tools, path).await.unwrap();
        SourceClip::new(
            Url::from_file_path(path).unwrap(),
            info.duration,
            info.pixel_size,
            range,
        )
    }

    /// A Seedance Edit came back silent and shorter than its source (3.71 s from 4.02 s), and a background cut-out
    /// silent: the source's sound goes on cut to the result's length, never padded past it.
    #[tokio::test]
    async fn a_shorter_silent_result_gets_the_sources_sound_cut_to_its_length() {
        let Some(tools) = tools() else { return };
        for (mode, operation) in [
            (Mode::Edit, None),
            (Mode::Extend, Some(PostAction::RemoveBackground)),
        ] {
            let folder = dir();
            let picture = Video::new(4).write(&tools, folder.path()).await;
            let source = adding_tone(&tools, &picture, folder.path()).await;
            let downloaded = Video::new(3).write(&tools, folder.path()).await;
            let result = folder.path().join(".result.partial");
            std::fs::rename(&downloaded, &result).unwrap();
            let output = folder.path().join(".result.audio.mp4");
            let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));
            let mut spec = JobSpec::new(
                mode,
                VideoModel::SEEDANCE25,
                Some(clip(&tools, &source, ClosedRange::new(0.0, 4.0)).await),
                Resolution::P480,
            );
            spec.operation = operation.map(|action| PostOperation {
                action,
                source_job_id: None,
            });

            assert!(preparer.restore_audio(&spec, &result, &output).await.unwrap());

            let report = probe_streams(&tools, output.as_os_str()).await.unwrap();
            let video_length = report.video.unwrap().time.duration;
            let audio_length = report.audio.unwrap().duration;
            assert!((video_length - 3.0).abs() < 0.05, "{video_length}");
            assert!(
                audio_length <= video_length + 0.05 && audio_length > 2.9,
                "{audio_length}"
            );
        }
    }

    #[tokio::test]
    async fn a_faded_sound_ends_silent() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let picture = Video::new(3).size(320, 240).write(&tools, folder.path()).await;
        let source = adding_tone(&tools, &picture, folder.path()).await;
        let result = Video::new(2).size(320, 240).write(&tools, folder.path()).await;
        let output = folder.path().join("faded.mp4");

        let restored = restore(
            &tools,
            AudioOption::FadeOut,
            ClosedRange::new(0.0, 3.0),
            &source,
            &result,
            &output,
        )
        .await
        .unwrap();

        assert!(restored);
        let report = probe_streams(&tools, output.as_os_str()).await.unwrap();
        assert!(report.audio.is_some());
    }

    #[tokio::test]
    async fn silent_source_leaves_the_result_alone() {
        let Some(tools) = tools() else { return };
        let folder = dir();
        let source = Video::new(6).write(&tools, folder.path()).await;
        let downloaded = Video::new(4).write(&tools, folder.path()).await;
        // Downloads arrive without an extension.
        let result = folder.path().join(".result.partial");
        std::fs::rename(&downloaded, &result).unwrap();
        let output = folder.path().join(".result.audio.mp4");
        let preparer = MediaPreparer::new(tools.clone(), folder.path().join("prepared"));

        for option in AudioOption::ALL {
            let mut spec = JobSpec::new(
                Mode::MotionTransfer,
                VideoModel::GENJUTSU,
                Some(clip(&tools, &source, ClosedRange::new(1.0, 5.0)).await),
                Resolution::P480,
            );
            spec.audio = option;
            assert!(!preparer.restore_audio(&spec, &result, &output).await.unwrap());
        }
        assert!(!output.exists());
        let mut names: Vec<String> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected = vec![
            ".result.partial".to_owned(),
            source.file_name().unwrap().to_string_lossy().into_owned(),
        ];
        expected.sort();
        assert_eq!(names, expected);
    }
}
