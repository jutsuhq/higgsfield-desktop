//! Port of `PostOperationProviderTests.swift`: the operations on a result on the account (issue #41), with the tool
//! definitions of the catalog snapshot; every call is checked against its `inputSchema`. Also the REST API's refusal
//! of every operation (`APIPostOperations`).

use jutsu_core::{
    AccountVoice, ClosedRange, Cost, DubLanguage, GenerationProvider, JobSpec, Mode, PixelSize, PortError,
    PostAction, PostOperation, ProviderError, ReframeAspect, RemoteAsset, RemoteInputs, Resolution,
    SourceClip, UpscaleEngine, UpscaleFrameRate, UpscaleOptions, UpscaleTarget, VideoModel, VoiceChoice,
    VoiceLibrary,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use super::support::account::{calls, full_set, schema_violations};
use super::support::agent_d::{self, file_url, names, params, same_list, spec as base_spec};
use super::support::catalog;
use super::support::stub::Stub;
use crate::HiggsfieldMcpProvider;
use crate::api_model::{ApiModel, needs_account};
use crate::mapping::not_available;

const RESULT_ID: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";
const UPLOAD_ID: &str = "5b7d1e90-2a4c-4e8f-9d36-7c1a2b3e4f50";

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.extend(
        [
            "upscale_video",
            "remove_background",
            "reframe",
            "dubbing",
            "voice_change",
            "list_voices",
        ]
        .map(catalog::tool),
    );
    tools
}

fn uploaded() -> RemoteInputs {
    RemoteInputs {
        video: Some(RemoteAsset {
            url: file_url("/in.mp4"),
            id: Some(UPLOAD_ID.to_owned()),
        }),
        images: Vec::new(),
        audio: None,
    }
}

fn no_inputs() -> RemoteInputs {
    RemoteInputs {
        video: None,
        images: Vec::new(),
        audio: None,
    }
}

fn spec_of(action: PostAction, job_id: Option<&str>, seconds: f64) -> JobSpec {
    let mut spec = base_spec(Mode::MotionTransfer);
    if let Some(source) = &mut spec.source {
        source.range = ClosedRange::new(0.0, seconds);
    }
    spec.operation = Some(PostOperation {
        action,
        source_job_id: job_id.map(str::to_owned),
    });
    spec
}

fn spec(action: PostAction) -> JobSpec {
    spec_of(action, Some(RESULT_ID), 10.0)
}

fn upscale() -> PostAction {
    PostAction::Upscale(UpscaleOptions {
        target: UpscaleTarget::K4,
        frame_rate: UpscaleFrameRate::Fps60,
        engine: UpscaleEngine::Standard,
        ..UpscaleOptions::default()
    })
}

fn voice() -> VoiceChoice {
    VoiceChoice {
        id: "v-1".to_owned(),
        kind: "element".to_owned(),
        name: "Mine".to_owned(),
    }
}

async fn provider(
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    agent_d::provider(tools(), call).await
}

fn credits(amount: i64, is_estimate: bool) -> Cost {
    Cost::credits(Decimal::from(amount), is_estimate)
}

fn unavailable() -> Result<Cost, PortError> {
    Err(PortError::Provider(not_available()))
}

/// Quoted through `generate_video` by the result's length, resolution and frame rate.
#[tokio::test]
async fn upscale_names_the_result_by_its_job_id_and_is_quoted_by_its_length() {
    let (provider, stub) = provider(|name, _| {
        if name == "upscale_video" {
            json!({"results": [{"id": "job-2"}], "request_id": "req-2"})
        } else {
            json!({"cost": {"credits": 15}})
        }
    })
    .await;
    let spec = spec(upscale());

    assert_eq!(provider.estimate(&spec).await, Ok(credits(15, true)));
    assert_eq!(
        provider.server_estimate(&spec, &no_inputs()).await,
        Ok(Some(credits(15, false)))
    );
    assert_eq!(provider.submit_is_idempotent(&spec).await, Ok(false));
    let remote = provider
        .submit(&spec, &no_inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(remote.request_id, "job-2");
    assert_eq!(remote.correlation_id.as_deref(), Some("req-2"));
    assert_eq!(
        names(&stub).await,
        vec![
            json!("generate_video"),
            json!("generate_video"),
            json!("upscale_video")
        ]
    );
    let quote = json!({
        "model": "bytedance_video_upscale", "get_cost": true, "duration": 10, "resolution": "4k", "fps": 60,
    });
    assert!(same_list(
        &params(&stub).await,
        &[
            quote.clone(),
            quote,
            json!({
                "video_id": RESULT_ID, "provider": "bytedance", "resolution": "4k",
                "fps": 60, "width": 1280, "height": 720, "preset": "aigc",
            }),
        ]
    ));
}

#[tokio::test]
async fn topaz_at4_k_sends2160p_and_neither_size_nor_rate() {
    let (provider, stub) = provider(|_, _| json!({"results": [{"id": "job-3"}]})).await;
    let spec = spec(PostAction::Upscale(UpscaleOptions {
        target: UpscaleTarget::K4,
        frame_rate: UpscaleFrameRate::Fps60,
        engine: UpscaleEngine::Topaz,
        ..UpscaleOptions::default()
    }));

    provider
        .submit(&spec, &no_inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert!(same_list(
        &params(&stub).await,
        &[json!({"video_id": RESULT_ID, "provider": "topaz", "resolution": "2160p"})]
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn upscales_the_tool_cant_take_are_refused_before_sending() {
    let (provider, stub) = provider(|_, _| json!({})).await;
    let mut unmeasured = spec(upscale());
    if let Some(source) = &mut unmeasured.source {
        source.pixel_size = PixelSize::new(0, 0);
    }
    let topaz_2k = spec(PostAction::Upscale(UpscaleOptions {
        target: UpscaleTarget::K2,
        engine: UpscaleEngine::Topaz,
        ..UpscaleOptions::default()
    }));

    for spec in [unmeasured, topaz_2k] {
        assert_eq!(provider.estimate(&spec).await, unavailable(), "{spec:?}");
    }
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn a_kept_frame_rate_of_a_stored_job_is_left_out() {
    let (provider, stub) = provider(|_, _| json!({"results": [{"id": "job-3"}]})).await;
    let spec = spec(PostAction::Upscale(UpscaleOptions {
        target: UpscaleTarget::P1080,
        frame_rate: UpscaleFrameRate::Keep,
        ..UpscaleOptions::default()
    }));

    provider
        .submit(&spec, &no_inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert!(
        params(&stub)
            .await
            .first()
            .and_then(|sent| sent.get("fps"))
            .is_none()
    );
}

#[tokio::test]
async fn remove_background_is_quoted_and_runs_on_the_upload() {
    let (provider, stub) = provider(|name, _| {
        if name == "generate_video" {
            json!({"cost": {"credits": 1}})
        } else {
            json!({"results": [{"id": "job-4"}]})
        }
    })
    .await;
    let spec = spec(PostAction::RemoveBackground);

    assert_eq!(provider.estimate(&spec).await, Ok(credits(1, true)));
    let remote = provider.submit(&spec, &uploaded(), Uuid::new_v4()).await.unwrap();

    assert_eq!(remote.request_id, "job-4");
    assert!(same_list(
        &params(&stub).await,
        &[
            json!({"model": "video_background_remover", "get_cost": true, "duration": 10}),
            json!({"media_id": UPLOAD_ID, "media_type": "video"}),
        ]
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn reframe_is_quoted_before_upload_even_for_an_uploaded_result() {
    let (provider, stub) = provider(|_, arguments| {
        if arguments["params"]["get_cost"] == true {
            json!({"cost": {"credits": 41}})
        } else {
            json!({"results": [{"id": "job-5"}]})
        }
    })
    .await;
    let spec = spec_of(PostAction::Reframe(ReframeAspect::Square), None, 20.0);

    assert_eq!(provider.estimate(&spec).await, Ok(credits(41, true)));
    assert_eq!(
        provider.server_estimate(&spec, &uploaded()).await,
        Ok(Some(credits(41, false)))
    );
    provider.submit(&spec, &uploaded(), Uuid::new_v4()).await.unwrap();

    let quote = json!({"get_cost": true, "duration_seconds": 20, "resolution": "720p"});
    assert!(same_list(
        &params(&stub).await,
        &[
            quote.clone(),
            quote,
            json!({
                "medias": [{"role": "video", "value": UPLOAD_ID}],
                "aspect_ratio": "1:1",
                "duration_seconds": 20, "resolution": "720p",
            }),
        ]
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

/// Up to 15 s the reframe goes without `resolution`, so a 1080p result stays 1080p; the quote, which needs a
/// resolution, names the result's.
#[tokio::test]
async fn a_short_reframe_keeps_the_results_resolution() {
    let (provider, stub) = provider(|_, arguments| {
        if arguments["params"]["get_cost"] == true {
            json!({"cost": {"credits": 30}})
        } else {
            json!({"results": [{"id": "job-5"}]})
        }
    })
    .await;
    let mut spec = spec_of(
        PostAction::Reframe(ReframeAspect::Portrait),
        Some(RESULT_ID),
        12.0,
    );
    spec.resolution = Resolution::P1080;

    assert_eq!(provider.estimate(&spec).await, Ok(credits(30, true)));
    provider
        .submit(&spec, &no_inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert!(same_list(
        &params(&stub).await,
        &[
            json!({"get_cost": true, "duration_seconds": 12, "resolution": "1080p"}),
            json!({
                "medias": [{"role": "video", "value": RESULT_ID}],
                "aspect_ratio": "9:16",
            }),
        ]
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_result_over_a_minute_cant_be_reframed() {
    let (provider, stub) = provider(|_, _| json!({})).await;
    assert_eq!(
        provider
            .estimate(&spec_of(
                PostAction::Reframe(ReframeAspect::Portrait),
                Some(RESULT_ID),
                61.0
            ))
            .await,
        unavailable()
    );
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

/// A 7 s dub is 21 credits, as Higgsfield billed one on 2026-10-15; a price question that fails leaves the price
/// unknown.
#[tokio::test]
async fn dub_and_change_voice_are_quoted_by_length_and_name_the_result() {
    let (provider, stub) = provider(|name, arguments| {
        if name != "generate_video" {
            return json!({"results": [{"id": "job-6"}]});
        }
        if arguments["params"]["model"] == "dubbing" {
            json!({"cost": {"credits": 21}})
        } else {
            json!({"error": "failed"})
        }
    })
    .await;
    let dub = spec_of(PostAction::Dub(DubLanguage::Japanese), Some(RESULT_ID), 7.04);
    let voice = spec(PostAction::ChangeVoice(voice()));

    assert_eq!(provider.estimate(&dub).await, Ok(credits(21, true)));
    assert_eq!(provider.estimate(&voice).await, Ok(Cost::UNKNOWN_CREDITS));
    provider.submit(&dub, &no_inputs(), Uuid::new_v4()).await.unwrap();
    provider
        .submit(&voice, &uploaded(), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(
        names(&stub).await,
        vec![
            json!("generate_video"),
            json!("generate_video"),
            json!("dubbing"),
            json!("voice_change")
        ]
    );
    assert!(same_list(
        &params(&stub).await,
        &[
            json!({"model": "dubbing", "get_cost": true, "duration": 7}),
            json!({"model": "voice_change", "get_cost": true, "duration": 10}),
            json!({"video_id": RESULT_ID, "target_language": "jpn"}),
            json!({"video_id": UPLOAD_ID, "voice_id": "v-1", "voice_type": "element"}),
        ]
    ));
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn voices_are_read_once_per_connection() {
    let (provider, stub) = provider(|name, arguments| {
        if name != "list_voices" {
            return json!({});
        }
        if arguments.get("cursor").is_none() {
            json!({
                "voices": [
                    {
                        "voice_id": "v-1", "voice_type": "preset", "name": "Amber",
                        "preview_url": "https://cdn.example/amber.mp3",
                    },
                ],
                "has_more": true, "next_cursor": "c-2",
            })
        } else {
            json!({"voices": [{"voice_id": "v-2", "voice_type": "element", "name": "Mine"}], "has_more": false})
        }
    })
    .await;

    let voices = provider.voices().await.unwrap();
    let again = provider.voices().await.unwrap();

    assert_eq!(
        voices,
        vec![
            AccountVoice {
                choice: VoiceChoice {
                    id: "v-1".to_owned(),
                    kind: "preset".to_owned(),
                    name: "Amber".to_owned()
                },
                preview_url: Some(Url::parse("https://cdn.example/amber.mp3").unwrap()),
            },
            AccountVoice {
                choice: VoiceChoice {
                    id: "v-2".to_owned(),
                    kind: "element".to_owned(),
                    name: "Mine".to_owned()
                },
                preview_url: None,
            },
        ]
    );
    assert_eq!(again, voices);
    assert_eq!(calls(&stub).await.len(), 2);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_tool_is_not_available() {
    let (provider, stub) = agent_d::provider(full_set(), |_, _| json!({})).await;

    assert!(matches!(
        provider.estimate(&spec(upscale())).await,
        Err(PortError::Provider(_))
    ));
    assert!(matches!(
        provider
            .estimate(&spec(PostAction::Dub(DubLanguage::French)))
            .await,
        Err(PortError::Provider(_))
    ));
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn an_operation_is_never_paid_from_the_unlimited_allowance() {
    let (provider, _stub) = provider(|_, _| json!({})).await;
    assert!(!provider.pays_unlimited(&spec(upscale())).await);
}

#[tokio::test]
async fn a_lost_answer_may_have_started_the_operation() {
    let (provider, _stub) = provider(|_, _| json!({"ok": true})).await;
    assert_eq!(
        provider
            .submit(&spec(upscale()), &no_inputs(), Uuid::new_v4())
            .await,
        Err(PortError::Provider(ProviderError::OutcomeUnknown))
    );
}

/// The REST API has none of the operations.
#[test]
fn an_api_key_refuses_operations() {
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        4.0,
        PixelSize::new(640, 360),
        ClosedRange::new(0.0, 4.0),
    );
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::GENJUTSU,
        Some(source),
        Resolution::P720,
    );
    spec.operation = Some(PostOperation {
        action: PostAction::RemoveBackground,
        source_job_id: None,
    });
    assert_eq!(ApiModel::new(&spec, &no_inputs()).err(), Some(needs_account()));
}
