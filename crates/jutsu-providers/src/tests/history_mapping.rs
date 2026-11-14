//! Port of `HistoryMappingTests.swift`.

use higgsfield::mcp::account::{MediaRole, PastGeneration, PastMedia};
use jutsu_core::{Mode, VideoModel};
use serde_json::json;

use super::support::agent_a::{media, past_generation, url};
use crate::history_mapping::{cursor, image, item, joined_cursor, kind_of_model, split_cursor};

/// The live model names of the account's history; Genjutsu's Style jobs report `restyle` (issue #123).
#[test]
fn history_ids_map_to_their_models_and_modes() {
    let kinds: Vec<Option<String>> = [
        "hf_mult_motion_control",
        "hf_mult_replace_object",
        "seedance_2_5",
        "kling3_0_motion_control",
    ]
    .iter()
    .map(|id| kind_of_model(id).map(|(mode, model)| format!("{} {}", mode.raw_value(), model.raw_value())))
    .collect();
    assert_eq!(
        kinds,
        [
            "motionTransfer genjutsu",
            "objectSwap genjutsu",
            "motionTransfer seedance25",
            "motionTransfer klingMotionControl",
        ]
        .map(|kind| Some(kind.to_owned()))
    );
    assert_eq!(
        kind_of_model("restyle").map(|(_, model)| model),
        Some(VideoModel::GENJUTSU)
    );
    assert_eq!(kind_of_model("hf_mult_restyle"), None);
    assert_eq!(kind_of_model("veo3"), None);
}

fn image_generation(kind: &str, status: &str, output: Option<&str>) -> PastGeneration {
    let mut generation = past_generation("img-1", kind, status, "seedream_5_0", output);
    generation.width = Some(1024);
    generation.height = Some(1024);
    "neon dance".clone_into(&mut generation.prompt);
    generation
}

const OUTPUT: Option<&str> = Some("https://cdn.example.com/out.png");

#[test]
fn completed_images_map_with_their_model_name() {
    let image = image(&image_generation("image", "completed", OUTPUT)).unwrap();
    assert_eq!(image.id, "img-1");
    assert_eq!(image.prompt, "neon dance");
    assert_eq!(image.model, "seedream_5_0");
    assert_eq!(image.aspect_ratio().as_deref(), Some("1:1"));
    assert_eq!(image.output_url, url("https://cdn.example.com/out.png"));
}

#[test]
fn unfinished_or_undownloadable_images_are_left_out() {
    assert_eq!(image(&image_generation("image", "in_progress", OUTPUT)), None);
    assert_eq!(image(&image_generation("video", "completed", OUTPUT)), None);
    assert_eq!(image(&image_generation("image", "completed", None)), None);
    assert_eq!(
        image(&image_generation(
            "image",
            "completed",
            Some("http://cdn.example.com/out.png")
        )),
        None
    );
}

#[test]
fn cursors_pack_both_streams_and_stay_readable() {
    assert_eq!(joined_cursor(None, None), None);
    let video = cursor(&json!("v1"));
    let image = cursor(&json!("i2"));
    let split = split_cursor(joined_cursor(video.as_ref(), image.as_ref()).as_ref());
    assert_eq!(split.0, Some(json!("v1")));
    assert_eq!(split.1, Some(json!("i2")));
    // One stream past its last page: only the other is asked again.
    let half = split_cursor(joined_cursor(None, image.as_ref()).as_ref());
    assert_eq!(half.0, None);
    assert_eq!(half.1, Some(json!("i2")));
    // A video-only cursor from before images were listed keeps working, even one shaped like an array.
    assert_eq!(split_cursor(video.as_ref()).0, Some(json!("v1")));
    assert_eq!(split_cursor(video.as_ref()).1, None);
    assert_eq!(split_cursor(cursor(&json!(["a", "b"])).as_ref()).1, None);
    assert_eq!(split_cursor(None), (None, None));
}

fn kling4(mode: Option<&str>, media: Vec<PastMedia>) -> PastGeneration {
    let mut generation = past_generation(
        "k4-1",
        "video",
        "completed",
        "kling4_0",
        Some("https://cdn.example/out.mp4"),
    );
    generation.duration = Some(6.0);
    "dance".clone_into(&mut generation.prompt);
    generation.resolution = Some("720p".to_owned());
    generation.media = media;
    generation.mode = mode.map(str::to_owned);
    generation
}

/// Kling 4.0 runs four of the app's jobs on one server model: its `params.mode` names which, and a clip from frames
/// comes back with them as frames. Without a `mode` the media decide, as before: a source or images are references.
#[test]
fn kling4_comes_back_in_its_mode() {
    let video = media(MediaRole::Video, "video-1", Some("https://cdn.example/in.mp4"));
    let image = media(MediaRole::Image, "image-1", Some("https://cdn.example/a.png"));
    let second = media(MediaRole::Image, "image-2", Some("https://cdn.example/b.png"));
    let start = media(MediaRole::StartFrame, "start", Some("https://cdn.example/s.png"));
    let end = media(MediaRole::EndFrame, "end", Some("https://cdn.example/e.png"));
    let ids = |item: &jutsu_core::HistoryItem| -> Vec<Option<String>> {
        item.inputs.images.iter().map(|image| image.id.clone()).collect()
    };

    let edit = item(&kling4(Some("video-edit"), vec![video.clone(), image.clone()])).unwrap();
    assert!(
        edit.mode == Mode::Edit
            && edit.spec(None, None).mode == Mode::Edit
            && edit.inputs.video.as_ref().and_then(|video| video.id.as_deref()) == Some("video-1")
    );
    assert_eq!(item(&kling4(Some("video-edit"), vec![image.clone()])), None);

    let reference = item(&kling4(
        Some("reference-to-video"),
        vec![video.clone(), image.clone()],
    ))
    .unwrap();
    assert!(reference.mode == Mode::MotionTransfer && !reference.references_are_frames);

    let text = item(&kling4(Some("text-to-video"), Vec::new())).unwrap();
    let text_spec = text.spec(None, None);
    assert!(
        text.mode == Mode::MotionTransfer && text_spec.source.is_none() && text_spec.references.is_empty()
    );

    let framed = item(&kling4(Some("image-to-video"), vec![end, start])).unwrap();
    assert_eq!(ids(&framed), [Some("start".to_owned()), Some("end".to_owned())]);
    assert!(framed.references_are_frames && framed.spec(None, None).uses_frames());
    // History may name the frames `image`, as it does references: `image-to-video` still reads them as frames.
    let renamed = item(&kling4(Some("image-to-video"), vec![image.clone(), second])).unwrap();
    assert!(
        ids(&renamed) == [Some("image-1".to_owned()), Some("image-2".to_owned())]
            && renamed.spec(None, None).uses_frames()
    );

    let unnamed = item(&kling4(None, vec![video, image])).unwrap();
    assert!(unnamed.mode == Mode::MotionTransfer && !unnamed.references_are_frames);
}

/// A recording the generation was voiced from comes back as its speech, by the link Higgsfield keeps it at; its
/// length is read once it is downloaded.
#[test]
fn a_speech_comes_back_with_the_inputs() {
    let image = media(MediaRole::Image, "image-1", Some("https://cdn.example/a.png"));
    let audio = media(MediaRole::Audio, "audio-1", Some("https://cdn.example/lines.mp3"));

    let item = item(&kling4(Some("reference-to-video"), vec![image, audio.clone()])).unwrap();

    assert_eq!(
        item.inputs
            .images
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>(),
        [Some("image-1".to_owned())]
    );
    assert_eq!(
        item.inputs.audio.as_ref().and_then(|audio| audio.id.as_deref()),
        Some("audio-1")
    );
    assert_eq!(
        item.spec(None, None).speech.map(|speech| speech.file_url),
        audio.url
    );
}
