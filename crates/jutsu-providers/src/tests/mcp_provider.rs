//! Port of `MCPProviderTests.swift`: the account connection's tools and answers as the live server gave them on
//! 2026-10-14, trimmed to the keys the provider reads.
//!
//! Not ported: `submitCancelledDuringTheCallHasAnUnknownOutcome` cancels a Swift `Task` while its call is out; a
//! Rust future is cancelled by dropping it, which leaves no error to check.

use std::collections::BTreeSet;

use higgsfield::core::HiggsfieldError;
use jutsu_core::{
    AspectRatio, AudioOption, Balance, CharacterOrientation, ClosedRange, CostUnit, GenerationProvider,
    JobSpec, Mode, ModelParameter, PixelSize, PortError, PreparedFile, ProviderError, ProviderKind,
    Rejection, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, Resolution, SeedanceBitrate, VideoModel,
};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use super::support::account::calls;
use super::support::agent_b::{
    answering, assert_same, asset, credits, generate_params, image, inputs, provider, provider_tools,
    quiet_provider, spec,
};
use super::support::mcp::{McpServer, Reply, tool};
use super::support::provider::make_provider;
use super::support::stub::{Stub, body_json, header, target};
use crate::mapping::not_available;
use crate::mcp_provider::{may_have_reached_server, not_cancellable};

fn not_available_error() -> PortError {
    PortError::Provider(not_available())
}

fn validation(message: &str) -> PortError {
    PortError::Provider(ProviderError::Validation {
        field: None,
        message: message.to_owned(),
    })
}

fn server(status: i64) -> PortError {
    PortError::Provider(ProviderError::Server { status })
}

fn outcome_unknown() -> PortError {
    PortError::Provider(ProviderError::OutcomeUnknown)
}

/// The `params` of every call, in order.
async fn all_params(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .map(|call| call["arguments"]["params"].clone())
        .collect()
}

#[tokio::test]
async fn full_tool_set_enables_motion_and_swap_in_credits() {
    let (provider, _stub) = quiet_provider(provider_tools()).await;
    let capabilities = provider.capabilities().await.unwrap();
    assert_eq!(
        capabilities.modes,
        BTreeSet::from([Mode::MotionTransfer, Mode::ObjectSwap, Mode::Edit, Mode::Extend])
    );
    let mut models: Vec<String> = capabilities
        .models
        .iter()
        .map(|model| model.raw_value().to_owned())
        .collect();
    models.sort();
    assert_eq!(
        models,
        [
            "cinemaStudio4",
            "geminiOmniFlash12",
            "genjutsu",
            "genjutsuLite",
            "intone",
            "kling40",
            "kling40Flash",
            "klingMotionControl",
            "klingO3VideoEdit",
            "minimaxH3MaxReference",
            "minimaxH3Reference",
            "seedance25",
            "wan30PrimeReference",
            "wan30Reference",
        ]
    );
    assert_eq!(
        capabilities.parameters,
        BTreeSet::from([
            ModelParameter::SeedanceBitrate,
            ModelParameter::SeedanceFrames,
            ModelParameter::KlingVideoBackground
        ])
    );
    assert_eq!(capabilities.cost_unit, CostUnit::Credits);
    assert_eq!(provider.kind(), ProviderKind::Account);
}

#[tokio::test]
async fn any_missing_job_tool_disables_every_mode() {
    let tools = provider_tools();
    let (provider, _stub) = quiet_provider(vec![tools[1].clone(), tools[2].clone(), tools[4].clone()]).await;
    assert!(provider.capabilities().await.unwrap().modes.is_empty());
    assert_eq!(provider.presets().await.unwrap_err(), not_available_error());
    assert_eq!(provider.balance().await.unwrap(), None);
}

#[tokio::test]
async fn schema_mismatch_disables_the_tool() {
    let tools = provider_tools();
    let status_by_request_id = tool("job_status", "Status", &["request_id"]);
    let (provider, _stub) = quiet_provider(vec![
        tools[1].clone(),
        tools[2].clone(),
        tools[3].clone(),
        status_by_request_id,
    ])
    .await;
    assert!(provider.capabilities().await.unwrap().modes.is_empty());
}

#[tokio::test]
async fn restyle_without_a_style_is_not_sent() {
    let (provider, stub) = quiet_provider(provider_tools()).await;
    let error = provider
        .submit(&spec(Mode::Restyle), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, not_available_error());
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn submit_sends_the_model_and_media_ids_and_reads_the_job() {
    let (provider, stub) = provider(provider_tools(), |name, _| match name {
        "generate_video" => json!({"results": [{"id": "job-1", "status": "queued"}], "request_id": "trace-1"}),
        "job_status" => json!({
            "generation": {"id": "job-1", "status": "completed", "results": {"rawUrl": "https://cdn.example/out.mp4"}},
        }),
        "balance" => json!({"credits": 2596.34, "subscription_plan_type": "ultra"}),
        _ => json!({}),
    })
    .await;
    let mut spec = spec(Mode::MotionTransfer);
    spec.prompt = "keep the background".into();
    let job = provider.submit(&spec, &inputs(), Uuid::new_v4()).await.unwrap();
    assert_eq!(job.request_id, "job-1");
    assert_eq!(job.correlation_id.as_deref(), Some("trace-1"));
    let output = Url::parse("https://cdn.example/out.mp4").unwrap();
    assert_eq!(
        provider.status(&job).await.unwrap(),
        RemoteStatus::Completed { video_url: output }
    );
    // 2596.34, rounded down
    assert_eq!(
        provider.balance().await.unwrap(),
        Some(Balance::Credits {
            credits: 2596,
            plan: Some("ultra".into())
        })
    );

    let calls = calls(&stub).await;
    let names: Vec<&Value> = calls.iter().map(|call| &call["name"]).collect();
    assert_eq!(
        names,
        [&json!("generate_video"), &json!("job_status"), &json!("balance")]
    );
    assert_same(
        &calls[0]["arguments"],
        &json!({
            "params": {
                "model": "hf_mult_motion_control",
                "medias": [
                    {"role": "video_references", "value": "video-1"},
                    {"role": "image_references", "value": "image-1"},
                ],
                "resolution": "720p", "use_unlim": false, "prompt": "keep the background",
            }
        }),
    );
    assert_same(&calls[1]["arguments"], &json!({"jobId": "job-1"}));
}

#[tokio::test]
async fn swap_uses_the_replace_object_model() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-2"}]})).await;
    provider
        .submit(&spec(Mode::ObjectSwap), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(all_params(&stub).await[0]["model"], "hf_mult_replace_object");
}

#[tokio::test]
async fn other_models_send_their_own_params() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-3"}]})).await;
    let mut seedance = spec(Mode::MotionTransfer);
    seedance.model = VideoModel::SEEDANCE25;
    let mut kling = spec(Mode::MotionTransfer);
    kling.model = VideoModel::KLING_MOTION_CONTROL;
    kling.resolution = Resolution::P1080;
    // `generate_audio` is a confirmed key: with a source, Seedance makes the sound when the job asks for it.
    let mut sounding = seedance.clone();
    sounding.audio = AudioOption::Generated;
    for spec in [&seedance, &kling, &sounding] {
        provider.submit(spec, &inputs(), Uuid::new_v4()).await.unwrap();
    }

    let params = all_params(&stub).await;
    assert_eq!(params[0]["model"], "seedance_2_5");
    assert_eq!(params[0]["mode"], "omni_reference");
    assert_same(&params[0]["duration"], &json!(10));
    assert_eq!(params[0]["resolution"], "720p");
    assert_eq!(params[0]["generate_audio"], false);
    assert_eq!(params[1]["model"], "kling3_0_motion_control");
    assert_eq!(params[1]["mode"], "pro");
    assert_same(&params[1]["duration"], &json!(10));
    assert!(params[1].get("resolution").is_none());
    assert_eq!(params[2]["mode"], "omni_reference");
    assert_eq!(params[2]["generate_audio"], true);
}

#[tokio::test]
async fn seedance_makes_a_clip_from_the_prompt_alone_or_with_images() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-4"}]})).await;
    let mut prompted = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P480,
    );
    prompted.prompt = "a fox in the snow".into();
    prompted.length = Some(12.0);
    prompted.aspect_ratio = Some(AspectRatio::Portrait);
    let nothing = RemoteInputs {
        video: None,
        images: Vec::new(),
        audio: None,
    };
    provider
        .submit(&prompted, &nothing, Uuid::new_v4())
        .await
        .unwrap();
    prompted.audio = AudioOption::Mute;
    prompted.aspect_ratio = None;
    let with_image = RemoteInputs {
        video: None,
        images: inputs().images,
        audio: None,
    };
    provider
        .submit(&prompted, &with_image, Uuid::new_v4())
        .await
        .unwrap();

    let params = all_params(&stub).await;
    assert_eq!(params[0]["mode"], "t2v");
    assert!(params[0].get("medias").is_none());
    assert_same(&params[0]["duration"], &json!(12));
    assert_eq!(params[0]["aspect_ratio"], "9:16");
    assert_eq!(params[0]["generate_audio"], true);
    assert_eq!(params[0]["prompt"], "a fox in the snow");
    assert_eq!(params[1]["mode"], "omni_reference");
    assert_same(
        &params[1]["medias"],
        &json!([{"role": "image_references", "value": "image-1"}]),
    );
    assert_eq!(params[1]["aspect_ratio"], "auto");
    assert_eq!(params[1]["generate_audio"], false);
    assert_eq!(provider.estimate(&prompted).await.unwrap(), credits("36", true));
}

#[tokio::test]
async fn other_models_serve_motion_only() {
    let (provider, stub) = quiet_provider(provider_tools()).await;
    let mut swap = spec(Mode::ObjectSwap);
    swap.model = VideoModel::SEEDANCE25;
    assert_eq!(
        provider
            .submit(&swap, &inputs(), Uuid::new_v4())
            .await
            .unwrap_err(),
        not_available_error()
    );
    assert_eq!(provider.estimate(&swap).await.unwrap_err(), not_available_error());
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

/// Edit and Extend as `models_explore get` of `seedance_2_5` names them (catalog snapshot of 2026-10-15), with the
/// clip as `video_references`: Edit's output follows the clip, so it sends no length; Extend sends the seconds it
/// adds and `extension_mode: "forward"`, and makes the new part's sound.
#[tokio::test]
async fn edit_and_extend_send_the_confirmed_modes() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-7"}]})).await;
    let mut edit = spec(Mode::Edit);
    edit.model = VideoModel::SEEDANCE25;
    edit.prompt = "make it night".into();
    let mut extend = edit.clone();
    extend.mode = Mode::Extend;
    extend.length = Some(6.0);
    let clip = RemoteInputs {
        video: inputs().video,
        images: Vec::new(),
        audio: None,
    };

    let modes = provider.capabilities().await.unwrap().modes;
    assert!(modes.contains(&Mode::Edit) && modes.contains(&Mode::Extend));
    provider.submit(&edit, &clip, Uuid::new_v4()).await.unwrap();
    provider.submit(&extend, &clip, Uuid::new_v4()).await.unwrap();

    let video = json!([{"role": "video_references", "value": "video-1"}]);
    assert_same(
        &Value::Array(generate_params(&stub).await),
        &json!([
            {
                "model": "seedance_2_5", "mode": "video_edit", "medias": video, "prompt": "make it night",
                "resolution": "720p", "generate_audio": false, "use_unlim": false,
            },
            {
                "model": "seedance_2_5", "mode": "video_extension", "extension_mode": "forward",
                "medias": video, "prompt": "make it night", "resolution": "720p", "duration": 6,
                "generate_audio": true, "use_unlim": false,
            },
        ]),
    );
}

/// Seedance with a source asks for the source's own shape: sent none, a 576×1024 source came back 1280×720. Edit
/// and Extend follow the clip and send none.
#[tokio::test]
async fn seedance_with_a_source_keeps_its_orientation() {
    for (width, height, ratio) in [(576, 1024, "9:16"), (1280, 720, "16:9"), (1000, 1000, "1:1")] {
        let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-8"}]})).await;
        let mut seedance = spec(Mode::MotionTransfer);
        seedance.model = VideoModel::SEEDANCE25;
        seedance.source.as_mut().unwrap().pixel_size = PixelSize { width, height };
        seedance.aspect_ratio = Some(AspectRatio::Landscape);
        provider
            .submit(&seedance, &inputs(), Uuid::new_v4())
            .await
            .unwrap();

        let params = all_params(&stub).await;
        assert_eq!(params[0]["mode"], "omni_reference");
        assert_eq!(params[0]["aspect_ratio"], ratio, "{width}×{height}");
    }
}

/// Seedance's frames and bitrate as `models_explore get` of `seedance_2_5` names them: the images as `start_image`
/// and `end_image` under `omni_reference` with the start frame's shape as `aspect_ratio` (left out, a 9:16 start
/// frame made a 16:9 clip live on 2026-10-15), and `bitrate_mode` only when picked (left out, the account's default
/// is `standard`).
#[tokio::test]
async fn seedance_sends_frames_and_the_picked_bitrate() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-6"}]})).await;
    let mut frames = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P720,
    );
    frames.references = vec![image(), image()];
    frames.length = Some(5.0);
    frames.aspect_ratio = Some(AspectRatio::Portrait);
    frames.bitrate = Some(SeedanceBitrate::High);
    frames.references_are_frames = true;
    let images = RemoteInputs {
        video: None,
        images: vec![asset("/a.jpg", "image-1"), asset("/b.jpg", "image-2")],
        audio: None,
    };

    assert!(
        provider
            .capabilities()
            .await
            .unwrap()
            .parameters
            .contains(&ModelParameter::SeedanceFrames)
    );
    assert_eq!(provider.estimate(&frames).await.unwrap(), credits("35", true));
    provider.submit(&frames, &images, Uuid::new_v4()).await.unwrap();
    frames.references_are_frames = false;
    frames.bitrate = None;
    provider.submit(&frames, &images, Uuid::new_v4()).await.unwrap();

    // The submits; the estimate's quote is the call with `get_cost`.
    let params: Vec<Value> = generate_params(&stub)
        .await
        .into_iter()
        .filter(|params| params.get("get_cost").is_none())
        .collect();
    assert_eq!(params.len(), 2);
    assert_same(
        &params[0],
        &json!({
            "model": "seedance_2_5", "mode": "omni_reference",
            "medias": [{"role": "start_image", "value": "image-1"}, {"role": "end_image", "value": "image-2"}],
            "resolution": "720p", "duration": 5, "generate_audio": true, "bitrate_mode": "high",
            "aspect_ratio": "9:16", "use_unlim": false,
        }),
    );
    assert!(params[1]["mode"] == "omni_reference" && params[1].get("bitrate_mode").is_none());
    assert_eq!(params[1]["aspect_ratio"], "9:16");
}

/// `models_explore get` of `kling3_0_motion_control` lists only `mode` and `background_source`: the account offers
/// neither the source's sound on the server nor the character's orientation, and sends neither.
#[tokio::test]
async fn kling_sends_no_sound_or_orientation() {
    let (provider, stub) = provider(provider_tools(), |_, _| json!({"results": [{"id": "job-8"}]})).await;
    let parameters = provider.capabilities().await.unwrap().parameters;
    assert!(
        !parameters.contains(&ModelParameter::KlingOriginalSound)
            && !parameters.contains(&ModelParameter::KlingOrientation)
    );
    let mut kling = spec(Mode::MotionTransfer);
    kling.model = VideoModel::KLING_MOTION_CONTROL;
    kling.audio = AudioOption::Original;
    kling.character_orientation = Some(CharacterOrientation::Image);
    provider.submit(&kling, &inputs(), Uuid::new_v4()).await.unwrap();

    assert_same(
        &generate_params(&stub).await[0],
        &json!({
            "model": "kling3_0_motion_control", "mode": "std", "duration": 10,
            "medias": [
                {"role": "video_references", "value": "video-1"},
                {"role": "image_references", "value": "image-1"},
            ],
            "use_unlim": false,
        }),
    );
}

/// Explicit quotes also protect rounding and duration clamping.
#[tokio::test]
async fn estimate_follows_the_model() {
    let cases = [
        (VideoModel::GENJUTSU, Resolution::P1080, 10.0, "200"),
        (VideoModel::SEEDANCE25, Resolution::P480, 10.0, "30"),
        (VideoModel::SEEDANCE25, Resolution::P1080, 10.0, "160"),
        (VideoModel::KLING_MOTION_CONTROL, Resolution::P720, 10.0, "18"),
        (VideoModel::KLING_MOTION_CONTROL, Resolution::P1080, 10.0, "28"),
        (VideoModel::SEEDANCE25, Resolution::P480, 5.5, "18"),
        (VideoModel::SEEDANCE25, Resolution::P480, 41.0, "90"),
        (VideoModel::KLING_MOTION_CONTROL, Resolution::P1080, 5.5, "18"),
    ];
    for (model, resolution, seconds, expected) in cases {
        let (provider, _stub) = quiet_provider(provider_tools()).await;
        let mut spec = spec(Mode::MotionTransfer);
        spec.model = model.clone();
        spec.resolution = resolution;
        spec.source.as_mut().unwrap().range = ClosedRange::new(0.0, seconds);
        assert_eq!(
            provider.estimate(&spec).await.unwrap(),
            credits(expected, true),
            "{model:?} {resolution:?} {seconds}"
        );
    }
}

#[tokio::test]
async fn declined_submit_carries_the_server_reason() {
    let (provider, _stub) = provider(provider_tools(), |_, _| json!({"error": "Not enough credits"})).await;
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, validation("Not enough credits"));
}

#[tokio::test]
async fn asset_without_media_id_is_refused() {
    let (provider, stub) = quiet_provider(provider_tools()).await;
    let inputs = RemoteInputs {
        video: Some(RemoteAsset::new(super::support::agent_b::file_url("/in.mp4"))),
        images: Vec::new(),
        audio: None,
    };
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(error, PortError::Provider(_)), "{error:?}");
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn status_maps_to_remote_status() {
    let cases = [
        (json!({"status": "queued"}), RemoteStatus::Queued),
        (json!({"status": "in_progress"}), RemoteStatus::InProgress),
        (
            json!({"status": "failed", "error": "Bad input"}),
            RemoteStatus::Failed {
                message: Some("Bad input".into()),
            },
        ),
        (
            json!({"status": "nsfw"}),
            RemoteStatus::Rejected(Rejection::ExplicitContent),
        ),
        (
            json!({"status": "ip_detected"}),
            RemoteStatus::Rejected(Rejection::ProtectedContent),
        ),
        (json!({"status": "canceled"}), RemoteStatus::Canceled),
    ];
    for (generation, expected) in cases {
        let answer = json!({"generation": generation});
        let (provider, _stub) = provider(provider_tools(), move |_, _| answer.clone()).await;
        assert_eq!(
            provider.status(&RemoteJob::new("job-1")).await.unwrap(),
            expected,
            "{generation}"
        );
    }
}

#[tokio::test]
async fn estimate_is_per_second_and_the_server_quote_is_exact() {
    let (provider, stub) = provider(
        provider_tools(),
        |_, _| json!({"cost": {"credits": 55, "credits_exact": 55.5}}),
    )
    .await;
    let spec = spec(Mode::ObjectSwap);
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("76", true));
    assert_eq!(
        provider.server_estimate(&spec, &inputs()).await.unwrap(),
        Some(credits("55.5", false))
    );
    assert_eq!(all_params(&stub).await[0]["get_cost"], true);
}

#[tokio::test]
async fn upload_puts_the_file_and_confirms_its_media_id() {
    let mcp = McpServer::new(provider_tools()).call(|name, _| {
        if name == "media_upload" {
            json!({
                "uploads": [{
                    "media_id": "media-1", "method": "PUT",
                    "upload_url": "https://upload.example/in.mp4?X-Signature=abc",
                    "url": "https://cdn.example/media-1.mp4",
                }]
            })
        } else {
            json!({"results": [{"media_id": "media-1", "status": "uploaded"}]})
        }
    });
    let stub = Stub::replying(move |request| {
        if request.method.as_str() == "PUT" {
            Reply::status(200)
        } else {
            mcp.respond(request)
        }
    })
    .await;
    let provider = make_provider(&stub, false);
    let mut file = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
    std::io::Write::write_all(&mut file, b"0123456789").unwrap();
    let file_name = file.path().file_name().unwrap().to_str().unwrap().to_owned();

    let asset = provider
        .upload(PreparedFile {
            file_url: Url::from_file_path(file.path()).unwrap(),
            content_type: "video/mp4".into(),
            byte_count: 10,
            content_hash: "h".into(),
        })
        .await
        .unwrap();

    let public_url = Url::parse("https://cdn.example/media-1.mp4").unwrap();
    assert_eq!(
        asset,
        RemoteAsset {
            url: public_url,
            id: Some("media-1".into())
        }
    );
    let requests = stub.requests().await;
    let put = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .unwrap();
    assert_eq!(
        target(put).as_str(),
        "https://upload.example/in.mp4?X-Signature=abc"
    );
    assert_eq!(header(put, "Content-Type"), Some("video/mp4"));
    assert_eq!(header(put, "Authorization"), None);
    let calls = calls(&stub).await;
    assert_same(
        &calls[0]["arguments"],
        &json!({"filename": file_name, "content_type": "video/mp4"}),
    );
    assert_same(
        &calls[calls.len() - 1]["arguments"],
        &json!({"media_id": "media-1", "type": "video"}),
    );
}

#[tokio::test]
async fn cancel_is_refused() {
    let (provider, _stub) = quiet_provider(provider_tools()).await;
    assert_eq!(
        provider.cancel(&RemoteJob::new("job-1")).await.unwrap_err(),
        not_cancellable()
    );
}

/// The MCP stub, except that every `generate_video` call answers `status`.
async fn failing_submit(status: u16) -> (crate::HiggsfieldMcpProvider, Stub) {
    let mcp = McpServer::new(provider_tools());
    let stub = Stub::replying(move |request| {
        if body_json(request)["params"]["name"] == "generate_video" {
            Reply::status(status)
        } else {
            mcp.respond(request)
        }
    })
    .await;
    (make_provider(&stub, false), stub)
}

#[tokio::test]
async fn ambiguous_submit_failure_is_not_retryable() {
    let (provider, stub) = failing_submit(502).await;
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, outcome_unknown());
    assert_eq!(calls(&stub).await.len(), 1);
    assert!(
        !provider
            .submit_is_idempotent(&spec(Mode::MotionTransfer))
            .await
            .unwrap()
    );
    assert_eq!(provider.owner().as_deref(), Some("user_1"));
}

#[tokio::test]
async fn tool_failures_preserve_whether_a_submit_can_be_retried() {
    let cases = [
        (Some(-32603), outcome_unknown()),
        (Some(-32000), outcome_unknown()),
        (None, outcome_unknown()),
        (Some(-32602), validation("Internal failure")),
    ];
    for (code, expected) in cases {
        let payload = match code {
            Some(code) => json!({"error": {"code": code, "message": "Internal failure"}}),
            None => {
                json!({"result": {"isError": true, "content": [{"type": "text", "text": "Internal failure"}]}})
            }
        };
        let stub = answering("generate_video", payload, provider_tools(), |_, _| json!({})).await;
        let provider = make_provider(&stub, false);
        let error = provider
            .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
            .await
            .unwrap_err();
        assert_eq!(error, expected, "{code:?}");
        assert_eq!(calls(&stub).await.len(), 1);
    }
}

/// Higgsfield's refusal before execution, as the live server words it, keeps its reason.
#[tokio::test]
async fn an_invalid_request_refusal_is_a_validation_error() {
    let message = "Invalid request: unknown model \"x\".";
    let payload = json!({"result": {"isError": true, "content": [{"type": "text", "text": message}]}});
    let stub = answering("generate_video", payload, provider_tools(), |_, _| json!({})).await;
    let provider = make_provider(&stub, false);
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, validation(message));
}

/// A poll of a job that may already be paid is retried when its answer can't be read, never ended.
#[tokio::test]
async fn an_unreadable_status_is_a_retryable_server_error() {
    let payloads = [
        json!({"result": {"isError": true, "content": [{"type": "text", "text": "Busy"}]}}),
        json!({"error": {"code": -32602, "message": "Invalid params"}}),
        json!({"result": {"content": [{"type": "text", "text": "done"}], "structuredContent": {}}}),
    ];
    for payload in payloads {
        let stub = answering("job_status", payload.clone(), provider_tools(), |_, _| json!({})).await;
        let provider = make_provider(&stub, false);
        assert_eq!(
            provider.status(&RemoteJob::new("job-1")).await.unwrap_err(),
            server(200),
            "{payload}"
        );
    }
}

/// A paid call that fails before it is sent can be retried: nothing reached the server.
#[tokio::test]
async fn a_submit_that_fails_before_sending_is_a_network_error() {
    let stub = Stub::replying(|request| {
        if body_json(request)["method"] == "initialize" {
            Reply::status(503)
        } else {
            Reply::not_found()
        }
    })
    .await;
    let provider = make_provider(&stub, false);
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, PortError::Provider(ProviderError::Network));
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
#[allow(clippy::cast_precision_loss, reason = "Swift's Double(Int.max)")]
async fn malformed_server_prices_are_errors() {
    for (price, variants) in [
        (-1.0, 1),
        (1e30, 1),
        (i64::MAX as f64, 1),
        (i64::MAX as f64 / 2.0, 4),
    ] {
        let (provider, _stub) = provider(
            provider_tools(),
            move |_, _| json!({"cost": {"credits_exact": price}}),
        )
        .await;
        let mut spec = spec(Mode::MotionTransfer);
        spec.variants = variants;
        assert_eq!(
            provider.server_estimate(&spec, &inputs()).await.unwrap_err(),
            server(200),
            "{price} × {variants}"
        );
    }
}

#[tokio::test]
async fn refused_submit_stays_as_is() {
    let (provider, _stub) = failing_submit(429).await;
    let error = provider
        .submit(&spec(Mode::MotionTransfer), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(error, server(429));
}

#[test]
fn ambiguous_failures() {
    let cases = [
        (HiggsfieldError::Network, true),
        (HiggsfieldError::Server { status: 500 }, true),
        (HiggsfieldError::Server { status: 200 }, true),
        (HiggsfieldError::Server { status: 404 }, false),
        (HiggsfieldError::Unauthorized, false),
        (HiggsfieldError::validation("x"), false),
    ];
    for (error, reached) in cases {
        assert_eq!(may_have_reached_server(&error), reached, "{error:?}");
    }
}
