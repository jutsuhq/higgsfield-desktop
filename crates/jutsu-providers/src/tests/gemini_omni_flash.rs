//! Port of `GeminiOmniFlashTests.swift`: Gemini Omni Flash 1.2 on the account only, `gemini_omni_flash_1_2`'s five
//! `mode`s against its `models_explore get` in the catalog snapshot of 2026-11-10. Higgsfield's REST catalog has no
//! Gemini model.

use higgsfield::mcp::account::{MediaRole, PastGeneration, PastMedia, VideoRequest};
use jutsu_core::{
    AspectRatio, GenerationProvider, JobSpec, Mode, PixelSize, PortError, Resolution, ValidationIssue,
    VideoModel,
};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::support::agent_c::kling4;
use super::support::agent_c::{
    account_provider, assert_json, assert_none, assert_params_keep_to_snapshot, check, column, estimated,
    generate_params, images, inputs, no_inputs, quote, source, submitted, url,
};
use crate::api_model::ApiModel;
use crate::history_mapping;

fn gemini() -> VideoModel {
    VideoModel::from_raw("geminiOmniFlash12").unwrap()
}

#[derive(Debug, Clone)]
struct Clip {
    images: usize,
    prompt: &'static str,
    seconds: f64,
    resolution: Resolution,
    aspect_ratio: Option<AspectRatio>,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            images: 0,
            prompt: "a fox in the snow",
            seconds: 12.0,
            resolution: Resolution::P720,
            aspect_ratio: Some(AspectRatio::Portrait),
        }
    }
}

impl Clip {
    fn spec(&self) -> JobSpec {
        let mut spec = JobSpec::new(Mode::MotionTransfer, gemini(), None, self.resolution);
        spec.references = images(self.images, "/tmp/ref.png");
        self.prompt.clone_into(&mut spec.prompt);
        spec.length = Some(self.seconds);
        spec.aspect_ratio = self.aspect_ratio;
        spec
    }
}

fn clip() -> JobSpec {
    Clip::default().spec()
}

fn frames(count: usize, seconds: f64) -> JobSpec {
    let mut spec = Clip {
        images: count,
        prompt: "",
        seconds,
        ..Clip::default()
    }
    .spec();
    spec.references_are_frames = true;
    spec
}

/// References with the source (Motion Transfer), an Edit of it, or an Extend adding `added` seconds.
#[derive(Debug, Clone)]
struct OnSource {
    mode: Mode,
    seconds: f64,
    added: Option<f64>,
    images: usize,
}

impl Default for OnSource {
    fn default() -> Self {
        Self {
            mode: Mode::MotionTransfer,
            seconds: 8.0,
            added: None,
            images: 1,
        }
    }
}

impl OnSource {
    fn spec(&self) -> JobSpec {
        let clip = source("/tmp/in.mp4", 60.0, PixelSize::new(1280, 720), self.seconds);
        let mut spec = JobSpec::new(self.mode, gemini(), Some(clip), Resolution::P1080);
        spec.references = images(self.images, "/tmp/ref.png");
        spec.prompt = "make it rain".to_owned();
        spec.length = self.added;
        spec
    }
}

fn on_source(mode: Mode, seconds: f64) -> JobSpec {
    OnSource {
        mode,
        seconds,
        ..OnSource::default()
    }
    .spec()
}

fn extend(seconds: f64, added: f64, images: usize) -> JobSpec {
    OnSource {
        mode: Mode::Extend,
        seconds,
        added: Some(added),
        images,
    }
    .spec()
}

/// Higgsfield's REST catalog has no Gemini model: an API key refuses it before anything is sent.
#[test]
fn an_api_key_refuses_it() {
    assert!(matches!(
        ApiModel::new(&clip(), &no_inputs()),
        Err(PortError::Provider(_))
    ));
}

/// A prompt alone, frames, references, an Edit and an Extend, each keeping to the `models_explore get`: the nearest
/// of 16:9 and 9:16 without a source, none with one; no length for Edit; the frames as `start_image` and
/// `end_image`; no `generate_audio`, as the model makes its sound on its own.
#[tokio::test]
async fn the_account_sends_each_mode() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    let mut wide = Clip {
        images: 1,
        seconds: 30.0,
        resolution: Resolution::K4,
        aspect_ratio: Some(AspectRatio::Cinema),
        ..Clip::default()
    }
    .spec();
    wide.prompt = "a cat".to_owned();
    let narrow = Clip {
        seconds: 15.0,
        resolution: Resolution::P360,
        ..Clip::default()
    };
    let jobs = [
        (narrow.spec(), no_inputs()),
        (frames(1, 8.0), inputs(None, vec![kling4::start()])),
        (frames(2, 8.0), inputs(None, vec![kling4::start(), kling4::end()])),
        (
            on_source(Mode::MotionTransfer, 14.0),
            inputs(Some(kling4::video()), vec![kling4::start()]),
        ),
        (wide, inputs(None, vec![kling4::start()])),
        (
            on_source(Mode::Edit, 12.0),
            inputs(Some(kling4::video()), vec![kling4::start()]),
        ),
        (extend(6.0, 20.0, 0), inputs(Some(kling4::video()), Vec::new())),
    ];
    for (spec, inputs) in &jobs {
        provider.submit(spec, inputs, Uuid::new_v4()).await.unwrap();
    }
    let params = generate_params(&stub).await;
    assert_eq!(params.len(), jobs.len());
    assert_params_keep_to_snapshot(&params);
    assert!(
        params.iter().all(
            |params| params["model"] == "gemini_omni_flash_1_2" && params.get("generate_audio").is_none()
        )
    );
    assert_json(
        &column(&params, "mode"),
        &json!([
            "text-to-video",
            "image-to-video",
            "image-to-video",
            "reference-to-video",
            "reference-to-video",
            "edit",
            "extend"
        ]),
    );
    assert_json(&column(&params, "duration"), &json!([15, 8, 8, 10, 10, null, 10]));
    assert_json(
        &column(&params, "aspect_ratio"),
        &json!(["9:16", "9:16", "9:16", null, "16:9", null, null]),
    );
    assert_json(
        &column(&params, "resolution"),
        &json!(["360p", "720p", "720p", "1080p", "4k", "1080p", "1080p"]),
    );
    assert_json(
        &params[1]["medias"],
        &json!([{"role": "start_image", "value": "image-1"}]),
    );
    assert_json(
        &params[2]["medias"],
        &json!([{"role": "start_image", "value": "image-1"}, {"role": "end_image", "value": "image-2"}]),
    );
    assert_json(
        &params[5]["medias"],
        &json!([
            {"role": "video_references", "value": "video-1"},
            {"role": "image_references", "value": "image-1"},
        ]),
    );
    assert_json(
        &params[6]["medias"],
        &json!([{"role": "video_references", "value": "video-1"}]),
    );
}

// MARK: Limits

/// 3–15 s from a prompt or a start frame alone; 3–10 s with references, a source or an end frame, and for what
/// Extend adds (and its clip); an Edit's clip 3–15 s. Up to six images.
#[test]
fn validation_keeps_to_the_schema() {
    let seconds = |images, seconds| {
        Clip {
            images,
            seconds,
            ..Clip::default()
        }
        .spec()
    };
    let too_long = ValidationIssue::DurationTooLong;
    assert_none(&check(&seconds(0, 15.0)));
    assert!(check(&seconds(0, 16.0)).contains(&too_long(16.0)));
    assert_none(&check(&frames(1, 15.0)));
    assert!(check(&frames(2, 12.0)).contains(&too_long(12.0)));
    assert_none(&check(&seconds(6, 10.0)));
    assert!(check(&seconds(1, 11.0)).contains(&too_long(11.0)));
    assert!(check(&seconds(7, 10.0)).contains(&ValidationIssue::TooManyReferences { maximum: 6 }));
    assert!(check(&on_source(Mode::MotionTransfer, 11.0)).contains(&too_long(11.0)));
    assert_none(&check(&on_source(Mode::Edit, 15.0)));
    assert!(check(&on_source(Mode::Edit, 16.0)).contains(&too_long(16.0)));
    let extend = |added| extend(6.0, added, 1);
    assert_none(&check(&extend(10.0)));
    assert!(check(&extend(11.0)).contains(&too_long(11.0)));
    assert!(check(&extend(2.0)).contains(&ValidationIssue::DurationTooShort(2.0)));
    let unprompted = Clip {
        prompt: "",
        ..Clip::default()
    };
    assert_eq!(check(&unprompted.spec()), [ValidationIssue::PromptRequired]);
    assert_none(&check(&frames(1, 8.0)));
}

// MARK: Prices

/// Until a quote: 2 credits a second at 720p, from Higgsfield's published dollar price, and Google's ratios for the
/// other tiers. A prompt alone is quoted before upload; media, Edit and Extend after.
#[test]
fn the_account_estimates_until_a_quote() {
    let at = |resolution| {
        Clip {
            seconds: 10.0,
            resolution,
            ..Clip::default()
        }
        .spec()
    };
    assert_eq!(estimated(&at(Resolution::P720)), Some(20));
    assert_eq!(estimated(&at(Resolution::P360)), Some(7));
    assert_eq!(estimated(&at(Resolution::P1080)), Some(30));
    assert_eq!(estimated(&at(Resolution::K4)), Some(60));
    assert_eq!(estimated(&on_source(Mode::Edit, 12.0)), Some(36));
    assert_eq!(estimated(&extend(8.0, 5.0, 0)), Some(15));

    let model = "gemini_omni_flash_1_2";
    let request = quote(&clip(), model).unwrap();
    assert!(
        request.mode.as_deref() == Some(VideoRequest::KLING4_TEXT_TO_VIDEO)
            && request.duration == Some(12.0)
            && request.resolution.as_deref() == Some("720p")
    );
    let with_image = Clip {
        images: 1,
        ..Clip::default()
    };
    for spec in [
        frames(1, 8.0),
        with_image.spec(),
        on_source(Mode::MotionTransfer, 8.0),
        on_source(Mode::Edit, 8.0),
        extend(8.0, 5.0, 0),
    ] {
        assert!(quote(&spec, model).is_none());
    }
}

// MARK: History

fn generation(mode: &str, media: Vec<PastMedia>) -> PastGeneration {
    PastGeneration {
        id: "g-1".to_owned(),
        kind: "video".to_owned(),
        status: "completed".to_owned(),
        model: "gemini_omni_flash_1_2".to_owned(),
        created_at: OffsetDateTime::UNIX_EPOCH,
        is_liked: false,
        output_url: Some(url("https://cdn.example/out.mp4")),
        width: None,
        height: None,
        duration: Some(8.0),
        prompt: "rain".to_owned(),
        resolution: Some("4k".to_owned()),
        media,
        is_draft: false,
        draft_job_id: None,
        mode: Some(mode.to_owned()),
    }
}

/// `edit` and `extend` come back as Edit and Extend, its other modes as Motion Transfer; neither without its clip.
#[test]
fn history_names_its_modes() {
    let video = PastMedia {
        role: Some(MediaRole::Video),
        id: "video-1".to_owned(),
        url: Some(url("https://cdn.example/in.mp4")),
    };
    let edit = history_mapping::item(&generation("edit", vec![video.clone()])).unwrap();
    assert!(edit.mode == Mode::Edit && edit.spec(None, None).resolution == Resolution::K4);
    assert_eq!(
        history_mapping::item(&generation("extend", vec![video]))
            .unwrap()
            .mode,
        Mode::Extend
    );
    assert_eq!(
        history_mapping::item(&generation("text-to-video", Vec::new()))
            .unwrap()
            .mode,
        Mode::MotionTransfer
    );
    assert!(history_mapping::item(&generation("extend", Vec::new())).is_none());
    assert!(history_mapping::item(&generation("edit", Vec::new())).is_none());
}
