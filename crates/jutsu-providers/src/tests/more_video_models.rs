//! Port of `MoreVideoModelsTests.swift`: the reference-video models of issue #101, their REST bodies from the
//! documented schemas, and their `generate_video` arguments against the account's own schemas in the catalog snapshot
//! of 2026-11-10.

use higgsfield::api::ModelEndpoint;
use higgsfield::mcp::account::VideoEditOperation;
use jutsu_core::{
    AudioOption, CinemaControl, CinemaControls, GenerationProvider, JobSpec, Mode, ModelConstraintTable,
    ModelParameter, PixelSize, PortError, PostAction, PostOperation, ReframeAspect, RemoteAsset,
    RemoteInputs, Resolution, VideoControl, VideoModel,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::account::calls;
use super::support::agent_c::{
    account_provider, assert_json, assert_none, asset, body, credits, estimated, generate_calls, images,
    source, submitted,
};
use super::support::catalog;
use crate::account_model;
use crate::api_model::{ApiBody, ApiModel};

fn model(id: &str) -> VideoModel {
    VideoModel::from_raw(id).unwrap()
}

/// `MoreVideoModels.spec`, with the defaults Swift gives it.
#[derive(Debug, Clone)]
struct Spec {
    mode: Mode,
    seconds: f64,
    resolution: Resolution,
    size: PixelSize,
    audio: AudioOption,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            mode: Mode::MotionTransfer,
            seconds: 10.0,
            resolution: Resolution::P720,
            size: PixelSize::new(1280, 720),
            audio: AudioOption::Original,
        }
    }
}

impl Spec {
    fn of(&self, id: &str) -> JobSpec {
        let clip = source("/tmp/in.mp4", 40.0, self.size, self.seconds);
        let mut spec = JobSpec::new(self.mode, model(id), Some(clip), self.resolution);
        spec.references = images(1, "/tmp/ref.png");
        spec.prompt = "make it rain".to_owned();
        spec.audio = self.audio;
        spec
    }
}

fn spec(id: &str) -> JobSpec {
    Spec::default().of(id)
}

fn edit_at(id: &str, resolution: Resolution) -> JobSpec {
    Spec {
        mode: Mode::Edit,
        resolution,
        ..Spec::default()
    }
    .of(id)
}

fn at(id: &str, resolution: Resolution) -> JobSpec {
    Spec {
        resolution,
        ..Spec::default()
    }
    .of(id)
}

fn lasting(id: &str, seconds: f64) -> JobSpec {
    Spec {
        seconds,
        ..Spec::default()
    }
    .of(id)
}

fn sounding(id: &str, audio: AudioOption) -> JobSpec {
    Spec {
        audio,
        ..Spec::default()
    }
    .of(id)
}

fn inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(asset("https://cdn.example.com/in.mp4", "video-1")),
        images: vec![asset("https://cdn.example.com/a.jpg", "image-1")],
        audio: None,
    }
}

fn speech() -> RemoteAsset {
    asset("https://cdn.example.com/lines.mp3", "audio-1")
}

fn request(spec: &JobSpec) -> ApiModel {
    ApiModel::new(spec, &inputs()).unwrap()
}

fn medias() -> Value {
    json!([
        {"role": "video_references", "value": "video-1"},
        {"role": "image_references", "value": "image-1"},
    ])
}

/// The `params` of every call the stub got.
async fn all_params(stub: &super::support::stub::Stub) -> Vec<Value> {
    calls(stub)
        .await
        .iter()
        .map(|call| call["arguments"]["params"].clone())
        .filter(Value::is_object)
        .collect()
}

// MARK: REST

/// Kling O3 Edit: the source, the images and the quality; no length, the output follows the source.
#[test]
fn kling_o3_edit_sends_the_clip_the_images_and_the_quality() {
    let standard = request(&edit_at("klingO3VideoEdit", Resolution::P720));
    assert_eq!(standard.endpoint, ModelEndpoint::KlingO3VideoEdit);
    assert_eq!(
        body(&standard),
        r#"{"image_urls":["https://cdn.example.com/a.jpg"],"mode":"std","prompt":"make it rain","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
    let pro = request(&edit_at("klingO3VideoEdit", Resolution::P1080));
    assert!(body(&pro).contains(r#""mode":"pro""#));
}

/// Kling 2.6 Motion Control: 3.0's body, its quality the endpoint.
#[test]
fn kling26_is_the_motion_control_body_on_its_own_endpoints() {
    let standard = request(&spec("klingMotionControl26"));
    assert_eq!(standard.endpoint, ModelEndpoint::Kling26MotionControlStandard);
    assert!(matches!(standard.body, ApiBody::KlingMotion(_)));
    let pro = request(&at("klingMotionControl26", Resolution::P1080));
    assert_eq!(pro.endpoint, ModelEndpoint::Kling26MotionControlPro);
}

/// Wan 3.0 and 3.0 Prime: the selection's seconds, the resolution, the soundtrack switch; no frame, so the server's
/// `adaptive` follows the references.
#[test]
fn wan30_sends_the_length_resolution_and_sound() {
    for (id, endpoint) in [
        ("wan30Reference", ModelEndpoint::Wan30ReferenceToVideo),
        ("wan30PrimeReference", ModelEndpoint::Wan30PrimeReferenceToVideo),
    ] {
        let sent = request(&lasting(id, 12.5));
        assert_eq!(sent.endpoint, endpoint);
        assert_eq!(
            body(&sent),
            r#"{"duration":13,"generate_audio":false,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
        );
        let generated = request(&sounding(id, AudioOption::Generated));
        assert!(body(&generated).contains(r#""generate_audio":true"#));
    }
}

/// Wan 2.7: at most 10 s, 720p or 1080p, 16:9 or 9:16 by the source's frame, and no soundtrack switch.
#[test]
fn wan27_keeps_to_its_schema() {
    let portrait = Spec {
        seconds: 14.0,
        resolution: Resolution::P480,
        size: PixelSize::new(720, 1280),
        ..Spec::default()
    }
    .of("wan27Reference");
    let sent = request(&portrait);
    assert_eq!(sent.endpoint, ModelEndpoint::Wan27ReferenceToVideo);
    assert_eq!(
        body(&sent),
        r#"{"aspect_ratio":"9:16","duration":10,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
}

/// Cinema Studio 4.0: at least 4 s, 480p or 720p, the frame nearest the source's (it takes no "auto").
#[test]
fn cinema_studio_keeps_to_its_schema() {
    let spec = Spec {
        seconds: 2.0,
        resolution: Resolution::P1080,
        audio: AudioOption::Generated,
        ..Spec::default()
    }
    .of("cinemaStudio4");
    let sent = request(&spec);
    assert_eq!(sent.endpoint, ModelEndpoint::CinemaStudio4);
    assert_eq!(
        body(&sent),
        r#"{"aspect_ratio":"16:9","duration":4,"generate_audio":true,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
}

/// Cinema Studio's creative controls go in the body by their fields, only the ones the job sets.
#[test]
fn cinema_studio_sends_the_controls_it_sets() {
    let mut cinema = lasting("cinemaStudio4", 6.0);
    cinema.cinema_controls = CinemaControls::from_values(
        [
            (CinemaControl::CameraModel, "35mm-film"),
            (CinemaControl::CameraLens, "anamorphic"),
            (CinemaControl::CameraAperture, "f4-moderate"),
            (CinemaControl::CameraMovement, "dolly-in"),
            (CinemaControl::Genre, "noir"),
            (CinemaControl::Era, "1960s"),
            (CinemaControl::Light, "contre-jour"),
            (CinemaControl::Pacing, "calm"),
            (CinemaControl::ColorPalette, "neon-rain-at-midnight"),
        ]
        .map(|(control, value)| (control, value.to_owned())),
    );
    assert_eq!(
        body(&request(&cinema)),
        r#"{"aspect_ratio":"16:9","camera_aperture":"f4-moderate","camera_lens":"anamorphic","camera_model":"35mm-film","camera_movement":"dolly-in","color_palette":"neon-rain-at-midnight","duration":6,"era":"1960s","generate_audio":false,"genre":"noir","image_urls":["https://cdn.example.com/a.jpg"],"light":"contre-jour","pacing":"calm","prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
    // Another model with the same body sends none, even from a spec that has them.
    let mut wan = spec("wan30Reference");
    wan.cinema_controls = cinema.cinema_controls.clone();
    assert!(!body(&request(&wan)).contains("genre"));
}

/// MiniMax H3: 2K whatever the form's resolution, 5–15 s, no frame (the server's `auto`) and no soundtrack switch.
/// H3 Max isn't in the REST docs, so an API key can't run it.
#[test]
fn mini_max_h3_keeps_to_its_schema() {
    let short = request(&lasting("minimaxH3Reference", 3.0));
    assert_eq!(short.endpoint, ModelEndpoint::MinimaxH3ReferenceToVideo);
    assert_eq!(
        body(&short),
        r#"{"duration":5,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"2K","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
    let long = request(&lasting("minimaxH3Reference", 20.0));
    assert!(body(&long).contains(r#""duration":15"#));
    assert!(matches!(
        ApiModel::new(&at("minimaxH3MaxReference", Resolution::P768), &inputs()),
        Err(PortError::Provider(_))
    ));
}

// MARK: Account

/// Each catalog model the account runs sends only what `models_explore get` of its model lists: the roles, the
/// parameter names and their options and ranges, at each of its resolutions, at both ends of its length and with
/// each sound choice. The arguments also keep to `generate_video`'s `inputSchema`.
#[tokio::test]
async fn every_account_model_keeps_to_its_snapshot_schema() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    let mut sent = 0;
    let mut with_variants = 0;
    for model in VideoModel::all().into_iter().filter(account_model::runs) {
        for mode in model
            .modes()
            .into_iter()
            .filter(|mode| model.account_model_id(*mode).is_some())
        {
            let limits = model.limits(mode, true, &ModelConstraintTable::built_in());
            for resolution in model.resolutions_for(mode) {
                for seconds in [limits.duration.lower(), limits.duration.upper()] {
                    for audio in [AudioOption::Original, AudioOption::Generated] {
                        let mut spec = Spec {
                            mode,
                            seconds,
                            resolution,
                            audio,
                            ..Spec::default()
                        }
                        .of(model.raw_value());
                        // Variants (`count`) are a `generate_video` setting every account model takes.
                        spec.variants = if audio == AudioOption::Generated {
                            JobSpec::MAXIMUM_VARIANTS
                        } else {
                            1
                        };
                        if spec.variants > 1 {
                            with_variants += 1;
                        }
                        // A model that voices a speech takes it, and no source video.
                        let inputs = if model.declares(VideoControl::Speech) {
                            RemoteInputs {
                                video: None,
                                images: inputs().images,
                                audio: Some(speech()),
                            }
                        } else {
                            inputs()
                        };
                        provider.submit(&spec, &inputs, Uuid::new_v4()).await.unwrap();
                        sent += 1;
                    }
                }
            }
        }
    }
    let calls = generate_calls(&stub).await;
    assert_eq!(calls.len(), sent);
    assert_eq!(
        calls
            .iter()
            .filter(|call| catalog::same(&call["arguments"]["params"]["count"], &json!(4)))
            .count(),
        with_variants
    );
    // Kling 3.0's `duration` predates the snapshot: its `get` lists no length, and `generate_video` clamps or
    // ignores what a model doesn't take (`docs/higgsfield-api.md` 4.5).
    let known = "kling3_0_motion_control: duration isn't a parameter";
    for call in &calls {
        let arguments = &call["arguments"];
        assert_none(&catalog::violations(arguments, "generate_video"));
        let found: Vec<String> = catalog::violations_of_params(&arguments["params"])
            .into_iter()
            .filter(|found| found != known)
            .collect();
        assert!(found.is_empty(), "{found:?}");
    }
}

/// The new account models' arguments as sent.
#[tokio::test]
async fn the_new_account_models_send_their_params() {
    let (provider, stub) = account_provider(submitted("job-2")).await;
    let specs = [
        edit_at("klingO3VideoEdit", Resolution::P1080),
        spec("wan30Reference"),
        sounding("wan30PrimeReference", AudioOption::Generated),
        lasting("cinemaStudio4", 12.0),
    ];
    for spec in &specs {
        provider.submit(spec, &inputs(), Uuid::new_v4()).await.unwrap();
    }

    let params = all_params(&stub).await;
    assert_json(
        &params[0],
        &json!({
            "model": "kling_video_edit", "medias": medias(), "prompt": "make it rain", "mode": "pro",
            "use_unlim": false,
        }),
    );
    assert_json(
        &params[1],
        &json!({
            "model": "wan3_0", "medias": medias(), "prompt": "make it rain", "resolution": "720p", "duration": 10,
            "generate_audio": false, "use_unlim": false,
        }),
    );
    assert_eq!(params[2]["model"], "wan3_0_prime");
    assert_eq!(params[2]["generate_audio"], true);
    assert_json(
        &params[3],
        &json!({
            "model": "cinematic_studio_video_4_0", "medias": medias(), "prompt": "make it rain",
            "mode": "omni_reference", "resolution": "720p", "duration": 12, "generate_audio": false,
            "use_unlim": false,
        }),
    );
}

/// MiniMax H3 and H3 Max on the account: their own resolutions and the length, no `mode` or soundtrack, as their
/// `models_explore get` lists. Cinema Studio leaves its creative controls out there: the account names them by IDs
/// its catalog doesn't list.
#[tokio::test]
async fn mini_max_and_cinema_controls_on_the_account() {
    let (provider, stub) = account_provider(submitted("job-3")).await;
    let mut cinema = lasting("cinemaStudio4", 12.0);
    cinema.cinema_controls = CinemaControls::from_values([
        (CinemaControl::Genre, "noir".to_owned()),
        (CinemaControl::CameraModel, "8mm-film".to_owned()),
    ]);
    let max = Spec {
        seconds: 20.0,
        resolution: Resolution::P1080,
        ..Spec::default()
    }
    .of("minimaxH3MaxReference");
    let specs = [lasting("minimaxH3Reference", 8.0), max, cinema];
    for spec in &specs {
        provider.submit(spec, &inputs(), Uuid::new_v4()).await.unwrap();
    }

    let params = all_params(&stub).await;
    assert_json(
        &params[0],
        &json!({
            "model": "minimax_h3", "medias": medias(), "prompt": "make it rain", "resolution": "2K",
            "duration": 8, "use_unlim": false,
        }),
    );
    assert_json(
        &params[1],
        &json!({
            "model": "minimax_h3_max", "medias": medias(), "prompt": "make it rain", "resolution": "768p",
            "duration": 15, "use_unlim": false,
        }),
    );
    assert_json(
        &params[2],
        &json!({
            "model": "cinematic_studio_video_4_0", "medias": medias(), "prompt": "make it rain",
            "mode": "omni_reference", "resolution": "720p", "duration": 12, "generate_audio": false,
            "use_unlim": false,
        }),
    );
    assert!(!account_model::parameters().contains(&ModelParameter::CinemaControls));
}

/// A reframe takes 480p, 720p or 1080p: a MiniMax result's 2K or 768p is sent as the nearest of those.
#[test]
fn a_reframe_of_a_mini_max_result_takes_a_standard_resolution() {
    for (resolution, expected) in [
        (Resolution::K2, "1080p"),
        (Resolution::P768, "720p"),
        (Resolution::P480, "480p"),
    ] {
        let mut spec = at("minimaxH3MaxReference", resolution);
        spec.resolution = resolution;
        spec.operation = Some(PostOperation {
            action: PostAction::Reframe(ReframeAspect::Portrait),
            source_job_id: Some("job-1".to_owned()),
        });
        let Some(VideoEditOperation::Reframe { resolution: sent, .. }) = account_model::edit_operation(&spec)
        else {
            panic!("expected a reframe");
        };
        assert_eq!(sent, expected, "{resolution:?}");
    }
}

/// The fallback estimates of the new models, from the free quotes of 2026-10-15 (issue #113) and, for MiniMax, of
/// 2026-10-28, for when Higgsfield can't quote: 10 s. MiniMax H3 Max at 480p hasn't been quoted, so it has no
/// estimate: its price is unknown until Higgsfield quotes it.
#[test]
fn the_new_models_have_a_fallback_estimate() {
    for (id, resolution, expected) in [
        ("wan30Reference", Resolution::P480, Some(10)),
        ("wan30Reference", Resolution::P1080, Some(35)),
        ("wan30PrimeReference", Resolution::P720, Some(30)),
        ("wan30PrimeReference", Resolution::P1080, Some(60)),
        ("cinemaStudio4", Resolution::P480, Some(30)),
        ("cinemaStudio4", Resolution::P720, Some(70)),
        ("minimaxH3Reference", Resolution::K2, Some(20)),
        ("minimaxH3MaxReference", Resolution::P768, Some(25)),
        ("minimaxH3MaxReference", Resolution::P480, None),
    ] {
        assert_eq!(estimated(&at(id, resolution)), expected, "{id} {resolution:?}");
    }
}

/// Kling O3 Edit follows its source, so before upload it has only the estimate (12 s: std 18, pro 24).
#[tokio::test]
async fn kling_o3_edit_is_estimated_by_its_source() {
    let (provider, stub) = account_provider(json!({})).await;
    let edit = |resolution| {
        Spec {
            mode: Mode::Edit,
            seconds: 12.0,
            resolution,
            ..Spec::default()
        }
        .of("klingO3VideoEdit")
    };
    assert_eq!(
        provider.estimate(&edit(Resolution::P720)).await.unwrap(),
        credits("18", true)
    );
    assert_eq!(
        provider.estimate(&edit(Resolution::P1080)).await.unwrap(),
        credits("24", true)
    );
    assert_none(&calls(&stub).await);
}
