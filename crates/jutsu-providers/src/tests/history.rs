//! Port of `HistoryTests.swift`.

use higgsfield::mcp::account::{MediaRole, PastGeneration, PastMedia};
use jutsu_core::{
    ClosedRange, GenerationHistory, Mode, ModelConstraintTable, PixelSize, RemoteAsset, Resolution,
    Validation, ValidationIssue, VideoModel,
};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::support::account::{calls_of, full_set};
use super::support::agent_a::{mcp_provider, media, past_generation, url};
use super::support::mcp::{recorded_generations, tool};
use crate::history_mapping::{cursor, cursor_value, item};

pub(super) fn generations_tool() -> Value {
    tool(
        "show_generations",
        "Show the user's generations",
        &["type", "only_completed", "size", "cursor"],
    )
}

#[tokio::test]
async fn recorded_page_maps_to_an_object_swap() {
    let recorded = recorded_generations();
    let mut tools = full_set();
    tools.push(generations_tool());
    let (provider, stub) = mcp_provider(tools, move |name, _| {
        if name == "show_generations" {
            recorded.clone()
        } else {
            json!({})
        }
    })
    .await;

    let page = provider.history(None).await.unwrap();
    let next = page.next_cursor.clone().unwrap();
    provider.history(Some(next)).await.unwrap();

    let item = page.items.first().unwrap();
    assert_eq!(item.id, "<job id>");
    assert!(item.mode == Mode::ObjectSwap && item.model == VideoModel::GENJUTSU);
    let made = OffsetDateTime::from_unix_timestamp_nanos(1_791_032_606_742_653_000).unwrap();
    assert!(
        (item.created_at - made).abs() < time::Duration::microseconds(1),
        "{}",
        item.created_at
    );
    assert!(!item.is_liked);
    assert_eq!(item.duration, Some(8.0));
    assert_eq!(
        item.pixel_size,
        Some(PixelSize {
            width: 1080,
            height: 1920
        })
    );
    assert_eq!(item.resolution, Some(Resolution::P720));
    assert_eq!(item.prompt, "<1712 characters>");
    assert_eq!(
        item.inputs.video.as_ref().and_then(|video| video.id.as_deref()),
        Some("<media id>")
    );
    assert_eq!(item.inputs.images.len(), 3);
    assert_eq!(item.output_url.host_str(), Some("d8j0ntlcm91z4.cloudfront.net"));
    let arguments: Vec<Value> = calls_of(&stub, "show_generations")
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(
        arguments.first(),
        Some(&json!({"type": "video", "only_completed": true, "size": 24}))
    );
    assert_eq!(
        arguments.last().map(|arguments| arguments["cursor"].clone()),
        Some(json!("<cursor; value not recorded>"))
    );
}

const VIDEO_URL: &str = "https://cdn.example/in.mp4";
const IMAGE_URL: &str = "https://cdn.example/a.png";
const OUTPUT: &str = "https://cdn.example/out.mp4";

fn video() -> PastMedia {
    media(MediaRole::Video, "video-1", Some(VIDEO_URL))
}

fn image() -> PastMedia {
    media(MediaRole::Image, "image-1", Some(IMAGE_URL))
}

/// Swift's `generation(_:type:status:media:output:)`.
fn generation(
    model: &str,
    kind: &str,
    status: &str,
    media: Vec<PastMedia>,
    output: Option<&str>,
) -> PastGeneration {
    let mut generation = past_generation("job-1", kind, status, model, output);
    generation.width = Some(720);
    generation.height = Some(1280);
    generation.duration = Some(6.0);
    "dance".clone_into(&mut generation.prompt);
    generation.resolution = Some("720p".to_owned());
    generation.media = media;
    generation
}

/// A completed video with the source and an image.
fn plain(model: &str) -> PastGeneration {
    generation(model, "video", "completed", vec![video(), image()], Some(OUTPUT))
}

fn with_media(model: &str, media: Vec<PastMedia>) -> PastGeneration {
    generation(model, "video", "completed", media, Some(OUTPUT))
}

fn with_output(model: &str, output: Option<&str>) -> PastGeneration {
    generation(model, "video", "completed", vec![video(), image()], output)
}

fn with_state(model: &str, kind: &str, status: &str) -> PastGeneration {
    generation(model, kind, status, vec![video(), image()], Some(OUTPUT))
}

#[test]
fn history_keeps_only_videos_of_the_apps_models() {
    let genjutsu = |mode| Some((mode, VideoModel::GENJUTSU));
    let cases: Vec<(PastGeneration, Option<(Mode, VideoModel)>)> = vec![
        (plain("hf_mult_motion_control"), genjutsu(Mode::MotionTransfer)),
        (plain("hf_mult_replace_object"), genjutsu(Mode::ObjectSwap)),
        (
            plain("kling3_0_motion_control"),
            Some((Mode::MotionTransfer, VideoModel::KLING_MOTION_CONTROL)),
        ),
        (
            plain("seedance_2_5"),
            Some((Mode::MotionTransfer, VideoModel::SEEDANCE25)),
        ),
        (
            with_media("seedance_2_5", vec![image()]),
            Some((Mode::MotionTransfer, VideoModel::SEEDANCE25)),
        ),
        (
            with_media("seedance_2_5", Vec::new()),
            Some((Mode::MotionTransfer, VideoModel::SEEDANCE25)),
        ),
        (with_media("kling3_0_motion_control", vec![image()]), None),
        (with_media("hf_mult_motion_control", vec![image()]), None),
        (with_state("seedream_5_0_flash", "image", "completed"), None),
        (plain("kling3_0"), None),
        (with_media("veo3_1", Vec::new()), None),
        // A Style job is `model: "restyle"` (live run, 2026-10-15); the IDs guessed before don't count.
        (plain("restyle"), genjutsu(Mode::Restyle)),
        (with_media("restyle", vec![image()]), genjutsu(Mode::Restyle)),
        (plain("hf_mult_restyle"), None),
        (with_media("genjutsu_style", vec![video()]), None),
        (plain("higgsfield/genjutsu/restyle/v1.0"), None),
        (plain("Restyle"), None),
        // Still running, a Style job is `type: "image"`; only a completed video is kept.
        (with_state("restyle", "image", "in_progress"), None),
        (with_state("restyle", "video", "in_progress"), None),
        (with_state("restyle", "image", "completed"), None),
        (with_output("restyle", Some("http://cdn.example/out.mp4")), None),
        (with_state("hf_mult_motion_control", "image", "completed"), None),
        (with_state("hf_mult_motion_control", "video", "in_progress"), None),
        (with_output("hf_mult_motion_control", None), None),
        (
            with_media(
                "hf_mult_motion_control",
                vec![PastMedia {
                    role: Some(MediaRole::Video),
                    id: "video-1".to_owned(),
                    url: None,
                }],
            ),
            None,
        ),
        (
            with_output("hf_mult_motion_control", Some("file:///home/me/Movies/out.mp4")),
            None,
        ),
        (
            with_output("hf_mult_motion_control", Some("http://cdn.example/out.mp4")),
            None,
        ),
        (
            with_media(
                "hf_mult_motion_control",
                vec![media(
                    MediaRole::Video,
                    "video-1",
                    Some("file:///home/me/Movies/x.mov"),
                )],
            ),
            None,
        ),
    ];
    for (generation, kind) in cases {
        assert_eq!(
            item(&generation).map(|item| (item.mode, item.model)),
            kind,
            "{generation:?}"
        );
    }
}

#[test]
fn a_style_run_is_imported_as_a_restyle_that_needs_a_style_to_run_again() {
    let spec = item(&plain("restyle")).unwrap().spec(None, None);

    assert!(spec.mode == Mode::Restyle && spec.model == VideoModel::GENJUTSU);
    assert_eq!(
        spec.source.as_ref().map(|source| &source.file_url),
        Some(&url(VIDEO_URL))
    );
    assert!(
        Validation::check(&spec, &ModelConstraintTable::built_in())
            .contains(&ValidationIssue::PresetRequired)
    );
}

#[test]
fn history_inputs_become_a_job_that_reuses_the_media() {
    let item = item(&with_media(
        "hf_mult_motion_control",
        vec![image(), video(), image()],
    ))
    .unwrap();
    let spec = item.spec(
        Some(12.5),
        Some(PixelSize {
            width: 1920,
            height: 1080,
        }),
    );

    assert_eq!(
        item.inputs.video,
        Some(RemoteAsset {
            url: url(VIDEO_URL),
            id: Some("video-1".to_owned())
        })
    );
    assert_eq!(
        item.inputs
            .images
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>(),
        [Some("image-1".to_owned()), Some("image-1".to_owned())]
    );
    assert_eq!(spec.remote_inputs.as_ref(), Some(&item.inputs));
    let source = spec.source.clone().unwrap();
    assert_eq!(source.file_url, url(VIDEO_URL));
    assert_eq!(source.range, ClosedRange::new(0.0, 12.5));
    assert_eq!(
        source.pixel_size,
        PixelSize {
            width: 1920,
            height: 1080
        }
    );
    assert_eq!(
        spec.references
            .iter()
            .map(|image| image.file_url.clone())
            .collect::<Vec<_>>(),
        [url(IMAGE_URL), url(IMAGE_URL)]
    );
    assert!(spec.prompt == "dance" && spec.resolution == Resolution::P720 && !spec.is_draft);
}

#[test]
fn a_seedance_clip_made_without_a_video_keeps_its_length() {
    let item = item(&with_media("seedance_2_5", vec![image()])).unwrap();
    let spec = item.spec(None, None);

    assert_eq!(spec.source, None);
    assert_eq!(spec.length, Some(6.0));
    assert!((spec.duration() - 6.0).abs() < f64::EPSILON);
    assert_eq!(
        spec.references
            .iter()
            .map(|image| image.file_url.clone())
            .collect::<Vec<_>>(),
        [url(IMAGE_URL)]
    );
    assert_eq!(
        Validation::check(&spec, &ModelConstraintTable::built_in()),
        Vec::<ValidationIssue>::new()
    );
}

#[test]
fn history_cursor_round_trips_as_json() {
    let value = json!({"after": "abc", "offset": 24});
    let cursor_of_value = cursor(&value).unwrap();
    assert_eq!(cursor_value(&cursor_of_value), Some(value));
    assert_eq!(
        cursor_value(&cursor(&json!("opaque")).unwrap()),
        Some(json!("opaque"))
    );
}
