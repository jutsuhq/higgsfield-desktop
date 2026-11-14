//! Port of `IntoneTests.swift`: Higgsfield Intone on the account only, `intone` against its `models_explore get` in
//! the catalog snapshot of 2026-11-10. One or two characters say one speech recording; the speech sets the length.
//! Higgsfield's REST catalog has no Intone.

use higgsfield::mcp::account::{MediaRole, PastGeneration, PastMedia};
use jutsu_core::{
    AspectRatio, GenerationProvider, JobSpec, LocalAudio, Mode, PixelSize, PortError, RemoteAsset,
    RemoteInputs, Resolution, ValidationIssue, VideoModel,
};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::support::agent_c::kling4;
use super::support::agent_c::{
    account_provider, assert_json, assert_none, assert_params_keep_to_snapshot, asset, check, column,
    estimated, file, generate_params, images, no_inputs, quote, source, submitted, url,
};
use crate::account_model;
use crate::api_model::ApiModel;
use crate::history_mapping;

fn intone() -> VideoModel {
    VideoModel::from_raw("intone").unwrap()
}

fn speech(duration: f64) -> LocalAudio {
    LocalAudio {
        file_url: file("/tmp/lines.mp3"),
        duration,
    }
}

fn audio() -> RemoteAsset {
    asset("https://cdn.example.com/lines.mp3", "audio-1")
}

#[derive(Debug, Clone)]
struct Clip {
    characters: usize,
    speech: Option<LocalAudio>,
    resolution: Resolution,
    aspect_ratio: Option<AspectRatio>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            characters: 1,
            speech: Some(speech(9.0)),
            resolution: Resolution::P1080,
            aspect_ratio: Some(AspectRatio::Portrait),
        }
    }
}

impl Clip {
    fn spec(&self) -> JobSpec {
        let mut spec = JobSpec::new(Mode::MotionTransfer, intone(), None, self.resolution);
        spec.references = images(self.characters, "/tmp/face.png");
        spec.length = Some(5.0);
        spec.aspect_ratio = self.aspect_ratio;
        spec.speech.clone_from(&self.speech);
        spec
    }
}

fn clip() -> JobSpec {
    Clip::default().spec()
}

fn characters(count: usize) -> JobSpec {
    Clip {
        characters: count,
        ..Clip::default()
    }
    .spec()
}

fn saying(speech: Option<LocalAudio>) -> JobSpec {
    Clip {
        speech,
        ..Clip::default()
    }
    .spec()
}

fn at(resolution: Resolution) -> JobSpec {
    Clip {
        resolution,
        ..Clip::default()
    }
    .spec()
}

/// Higgsfield's REST catalog has no Intone: an API key refuses it before anything is sent.
#[test]
fn an_api_key_refuses_it() {
    assert!(matches!(
        ApiModel::new(&clip(), &no_inputs()),
        Err(PortError::Provider(_))
    ));
}

/// The images go as `character_images` and the speech as `audio_references`, with the resolution and the nearest of
/// the model's frames; no `duration`, `mode`, `generate_audio` or `bitrate_mode`.
#[tokio::test]
async fn the_account_sends_the_speakers_and_the_speech() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    let two = Clip {
        characters: 2,
        resolution: Resolution::P480,
        aspect_ratio: Some(AspectRatio::Auto),
        ..Clip::default()
    };
    let jobs = [
        (
            clip(),
            RemoteInputs {
                video: None,
                images: vec![kling4::start()],
                audio: Some(audio()),
            },
        ),
        (
            two.spec(),
            RemoteInputs {
                video: None,
                images: vec![kling4::start(), kling4::end()],
                audio: Some(audio()),
            },
        ),
    ];
    for (spec, inputs) in &jobs {
        provider.submit(spec, inputs, Uuid::new_v4()).await.unwrap();
    }
    let params = generate_params(&stub).await;
    assert_eq!(params.len(), jobs.len());
    assert_params_keep_to_snapshot(&params);
    assert!(params.iter().all(|params| params["model"] == "intone"));
    assert!(params.iter().all(|params| {
        ["duration", "mode", "generate_audio", "bitrate_mode"]
            .iter()
            .all(|key| params.get(key).is_none())
    }));
    assert_json(&column(&params, "resolution"), &json!(["1080p", "480p"]));
    assert_json(&column(&params, "aspect_ratio"), &json!(["9:16", null]));
    assert_json(
        &params[1]["medias"],
        &json!([
            {"role": "character_images", "value": "image-1"},
            {"role": "character_images", "value": "image-2"},
            {"role": "audio_references", "value": "audio-1"},
        ]),
    );
}

// MARK: Limits

/// One or two characters, a speech of 2–27.5 s (30 s with its tail), and no source video.
#[test]
fn validation_keeps_to_the_schema() {
    assert_none(&check(&clip()));
    assert_none(&check(&characters(2)));
    assert!(check(&characters(0)).contains(&ValidationIssue::TooFewReferences { minimum: 1 }));
    assert_eq!(
        check(&characters(3)),
        [ValidationIssue::TooManyReferences { maximum: 2 }]
    );
    assert_eq!(check(&saying(None)), [ValidationIssue::SpeechRequired]);
    assert_eq!(
        check(&saying(Some(speech(1.5)))),
        [ValidationIssue::SpeechTooShort(1.5)]
    );
    assert_eq!(
        check(&saying(Some(speech(28.0)))),
        [ValidationIssue::SpeechTooLong(28.0)]
    );
    assert_none(&check(&saying(Some(speech(27.5)))));
    let mut with_source = clip();
    with_source.source = Some(source("/tmp/in.mp4", 10.0, PixelSize::new(1280, 720), 10.0));
    assert_eq!(check(&with_source), [ValidationIssue::SourceNotTaken]);
}

// MARK: Prices

/// Until a quote, which comes only once the speech is uploaded: 1, 2 or 4 credits a second of the speech and its
/// 2.5 s tail.
#[test]
fn the_account_estimates_until_a_quote_after_upload() {
    assert_eq!(estimated(&clip()), Some(48));
    assert_eq!(estimated(&at(Resolution::P720)), Some(24));
    assert_eq!(estimated(&at(Resolution::P480)), Some(12));
    assert!(account_model::quotes_after_upload(&clip()));
    assert!(quote(&clip(), "intone").is_none());
}

// MARK: History

/// The speakers come back as the images, and the speech as long as the output without its tail.
#[test]
fn history_keeps_the_speakers_and_the_speech() {
    let media = |role, id: &str, link: &str| PastMedia {
        role: Some(role),
        id: id.to_owned(),
        url: Some(url(link)),
    };
    let generation = PastGeneration {
        id: "g-1".to_owned(),
        kind: "video".to_owned(),
        status: "completed".to_owned(),
        model: "intone".to_owned(),
        created_at: OffsetDateTime::UNIX_EPOCH,
        is_liked: false,
        output_url: Some(url("https://cdn.example/out.mp4")),
        width: None,
        height: None,
        duration: Some(11.5),
        prompt: String::new(),
        resolution: Some("720p".to_owned()),
        media: vec![
            media(MediaRole::Character, "image-1", "https://cdn.example/a.png"),
            media(MediaRole::Audio, "audio-1", "https://cdn.example/lines.mp3"),
        ],
        is_draft: false,
        draft_job_id: None,
        mode: None,
    };
    let item = history_mapping::item(&generation).unwrap();
    assert!(item.model == intone() && item.mode == Mode::MotionTransfer);
    let ids: Vec<_> = item
        .inputs
        .images
        .iter()
        .map(|image| image.id.as_deref())
        .collect();
    assert!(
        ids == [Some("image-1")]
            && item.inputs.audio.as_ref().and_then(|audio| audio.id.as_deref()) == Some("audio-1")
    );
    let spec = item.spec(None, None);
    assert!(
        spec.speech
            .as_ref()
            .is_some_and(|speech| (speech.duration - 9.0).abs() < f64::EPSILON)
    );
    assert!((spec.duration() - 11.5).abs() < f64::EPSILON);
    assert_none(&check(&spec));
}
