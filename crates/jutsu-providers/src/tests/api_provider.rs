//! Port of `APIProviderTests.swift`.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use higgsfield::api::HiggsfieldApiClient;
use jutsu_core::{
    AspectRatio, AudioOption, Capabilities, CharacterOrientation, ClosedRange, Cost, CostUnit,
    GenerationProvider, JobSpec, LocalImage, Mode, ModelParameter, PixelSize, PortError, PreparedFile,
    ProviderError, ProviderKind, Rejection, RemoteAsset, RemoteInputs, RemoteJob, RemoteStatus, Resolution,
    SeedanceBitrate, SourceClip, VideoModel,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::agent_a::{routes, url};
use super::support::mcp::Reply;
use super::support::provider::make_api_provider;
use super::support::stub::{Stub, body_json, body_text, dead_url};
use crate::HiggsfieldApiProvider;

const SUBMITTED: &str = r#"{"status": "queued", "request_id": "req-123",
 "status_url": "https://api.higgsfield.ai/requests/req-123/status",
 "cancel_url": "https://api.higgsfield.ai/requests/req-123/cancel"}"#;

const PRESET_ID: &str = "c2143317-f28d-4c3c-a0b8-39bd547e08a7";

fn preset_id() -> Uuid {
    Uuid::parse_str(PRESET_ID).unwrap()
}

fn inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(video()),
        images: vec![character()],
        audio: None,
    }
}

fn character() -> RemoteAsset {
    RemoteAsset::new(url("https://cdn.example.com/a.jpg"))
}

fn video() -> RemoteAsset {
    RemoteAsset::new(url("https://cdn.example.com/in.mp4"))
}

fn remote(video: Option<RemoteAsset>, images: Vec<RemoteAsset>) -> RemoteInputs {
    RemoteInputs {
        video,
        images,
        audio: None,
    }
}

fn usd(amount: &str, is_estimate: bool) -> Cost {
    Cost::usd(Decimal::from_str(amount).unwrap(), is_estimate)
}

fn validation(field: Option<&str>, message: &str) -> PortError {
    PortError::Provider(ProviderError::Validation {
        field: field.map(str::to_owned),
        message: message.to_owned(),
    })
}

/// A Genjutsu job of the first 12 s of a 1280×720 source.
fn spec(mode: Mode, preset_id: Option<Uuid>) -> JobSpec {
    let source = SourceClip::new(
        url("file:///tmp/source.mov"),
        60.0,
        PixelSize {
            width: 1280,
            height: 720,
        },
        ClosedRange::new(0.0, 12.0),
    );
    let mut spec = JobSpec::new(mode, VideoModel::GENJUTSU, Some(source), Resolution::P480);
    spec.preset_id = preset_id;
    spec
}

fn motion() -> JobSpec {
    spec(Mode::MotionTransfer, None)
}

fn kling() -> JobSpec {
    let mut kling = motion();
    kling.model = VideoModel::KLING_MOTION_CONTROL;
    kling
}

/// A Seedance job: from a 1080×1920 source when `has_source`, else a 9:16 clip of 8 s from the prompt.
fn seedance(has_source: bool, references: usize, audio: AudioOption) -> JobSpec {
    let source = SourceClip::new(
        url("file:///tmp/source.mov"),
        60.0,
        PixelSize {
            width: 1080,
            height: 1920,
        },
        ClosedRange::new(0.0, 12.0),
    );
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        has_source.then_some(source),
        Resolution::P720,
    );
    spec.references = (0..references)
        .map(|index| LocalImage::new(url(&format!("file:///tmp/ref{index}.jpg"))))
        .collect();
    "a fox in the snow".clone_into(&mut spec.prompt);
    spec.audio = audio;
    spec.length = (!has_source).then_some(8.0);
    spec.aspect_ratio = (!has_source).then_some(AspectRatio::Portrait);
    spec
}

fn submit_route(path: &str) -> (&'static str, String, Reply) {
    ("POST", path.to_owned(), Reply::json_text(SUBMITTED))
}

async fn provider(routes_list: Vec<(&'static str, String, Reply)>) -> (HiggsfieldApiProvider, Stub) {
    let stub = routes(routes_list).await;
    (make_api_provider(&stub, "key-id"), stub)
}

async fn first_body(stub: &Stub) -> String {
    body_text(stub.requests().await.first().unwrap())
}

async fn first_json(stub: &Stub) -> Value {
    body_json(stub.requests().await.first().unwrap())
}

#[tokio::test]
async fn submit_goes_to_the_mode_workflow_and_keeps_the_returned_urls() {
    for (mode, path) in [
        (Mode::MotionTransfer, "/higgsfield/genjutsu/motion-transfer/v1.0"),
        (Mode::ObjectSwap, "/higgsfield/genjutsu/object-swap/v1.0"),
        (Mode::Restyle, "/higgsfield/genjutsu/restyle/v1.0"),
    ] {
        let (provider, stub) = provider(vec![(
            "POST",
            path.to_owned(),
            Reply::json_text(SUBMITTED).header("X-Correlation-ID", "corr-1"),
        )])
        .await;

        let job = provider
            .submit(&spec(mode, Some(preset_id())), &inputs(), Uuid::new_v4())
            .await
            .unwrap();

        let mut expected = RemoteJob::new("req-123");
        expected.correlation_id = Some("corr-1".to_owned());
        expected.status_url = Some(url("https://api.higgsfield.ai/requests/req-123/status"));
        expected.cancel_url = Some(url("https://api.higgsfield.ai/requests/req-123/cancel"));
        expected.initial_status = Some(RemoteStatus::Queued);
        assert_eq!(job, expected, "{mode:?}");
        assert_eq!(
            first_body(&stub).await.contains("preset_id"),
            mode == Mode::Restyle,
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn restyle_without_preset_fails_before_sending() {
    let (provider, stub) = provider(Vec::new()).await;
    assert_eq!(
        provider
            .submit(&spec(Mode::Restyle, None), &inputs(), Uuid::new_v4())
            .await,
        Err(validation(Some("preset_id"), "Restyle needs a preset."))
    );
    assert!(stub.requests().await.is_empty());
}

/// Swift fails the status request with `URLError(.notConnectedToInternet)`; here it goes to a host nothing listens
/// on.
#[tokio::test]
async fn client_errors_arrive_as_provider_errors() {
    let (provider, _stub) = provider(vec![(
        "POST",
        "/higgsfield/genjutsu/motion-transfer/v1.0".to_owned(),
        Reply::json_text(r#"{"detail": {"code": "account_queue_full"}}"#)
            .with_status(429)
            .header("Retry-After", "30"),
    )])
    .await;
    let offline = HiggsfieldApiProvider::new(
        HiggsfieldApiClient::new("key-id", "key-secret").with_base_url(dead_url("/")),
        None,
    );

    assert_eq!(
        provider.submit(&motion(), &inputs(), Uuid::new_v4()).await,
        Err(PortError::Provider(ProviderError::ConcurrencyLimit {
            retry_after: Some(time::Duration::seconds(30))
        }))
    );
    assert_eq!(
        offline.status(&RemoteJob::new("req-1")).await,
        Err(PortError::Provider(ProviderError::Network))
    );
}

#[tokio::test]
async fn server_estimate_is_exact_usd() {
    let (provider, _stub) = provider(vec![(
        "POST",
        "/estimate/higgsfield/genjutsu/motion-transfer/v1.0".to_owned(),
        Reply::json_text(r#"{"credits": "61.000", "usd": "3.816"}"#),
    )])
    .await;

    let price = provider.server_estimate(&motion(), &inputs()).await.unwrap();

    assert_eq!(price, Some(usd("3.816", false)));
}

/// The upload URL is on another host, which the stub's client routes to the stub as well.
#[tokio::test]
async fn upload_returns_the_public_url() {
    let mut file = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
    std::io::Write::write_all(&mut file, b"fake video").unwrap();
    let (provider, _stub) = provider(vec![
        (
            "POST",
            "/files/generate-upload-url".to_owned(),
            Reply::json_text(
                r#"{"upload_url": "https://storage.example.com/bucket/in.mp4",
                    "public_url": "https://cdn.example.com/in.mp4"}"#,
            ),
        ),
        ("PUT", "/bucket/in.mp4".to_owned(), Reply::status(200)),
    ])
    .await;

    let asset = GenerationProvider::upload(
        &provider,
        PreparedFile {
            file_url: url::Url::from_file_path(file.path()).unwrap(),
            content_type: "video/mp4".to_owned(),
            byte_count: 10,
            content_hash: "abc".to_owned(),
        },
    )
    .await
    .unwrap();

    assert_eq!(asset, RemoteAsset::new(url("https://cdn.example.com/in.mp4")));
}

#[tokio::test]
async fn presets_become_style_presets() {
    let (provider, _stub) = provider(vec![(
        "GET",
        "/models/higgsfield/genjutsu/restyle/v1.0/presets".to_owned(),
        Reply::json_text(&format!(
            r#"{{"items": [{{"id": "{PRESET_ID}", "name": "Cel-Shaded",
                "preview_url": "https://cdn.higgsfield.ai/p.webp"}}]}}"#
        )),
    )])
    .await;

    let presets = provider.presets().await.unwrap();

    assert_eq!(
        presets.iter().map(|preset| preset.id).collect::<Vec<_>>(),
        [preset_id()]
    );
}

#[tokio::test]
async fn provider_reports_static_capabilities_and_list_price() {
    let stub = routes(Vec::new()).await;
    let provider = make_api_provider(&stub, "kid-1");
    // Genjutsu Lite's Object Swap has no REST endpoint.
    let lite = VideoModel::from_raw("genjutsuLite").unwrap();

    assert_eq!(GenerationProvider::kind(&provider), ProviderKind::ApiKey);
    assert_eq!(GenerationProvider::owner(&provider).as_deref(), Some("kid-1"));
    let mut expected = Capabilities::new(
        Mode::ALL.iter().copied().collect(),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Usd,
        None,
    );
    expected.models = VideoModel::all()
        .into_iter()
        .filter(VideoModel::runs_with_api_key)
        .collect();
    expected.parameters = ModelParameter::ALL
        .iter()
        .copied()
        .filter(|parameter| *parameter != ModelParameter::KlingVideoBackground)
        .collect();
    expected.unserved_modes = BTreeMap::from([(lite, BTreeSet::from([Mode::ObjectSwap]))]);
    assert_eq!(provider.capabilities().await.unwrap(), expected);
    assert_eq!(provider.estimate(&motion()).await.unwrap(), usd("3.816", false));
    assert_eq!(provider.balance().await.unwrap(), None);
    assert!(provider.submit_is_idempotent(&motion()).await.unwrap());
}

#[tokio::test]
async fn provider_polls_and_cancels_by_request_id() {
    let (provider, _stub) = provider(vec![
        (
            "GET",
            "/requests/req-7/status".to_owned(),
            Reply::json_text(r#"{"status": "nsfw", "request_id": "req-7"}"#),
        ),
        (
            "POST",
            "/requests/req-7/cancel".to_owned(),
            Reply::json_text(r#"{"detail": "Not queued"}"#).with_status(400),
        ),
    ])
    .await;
    let job = RemoteJob::new("req-7");

    assert_eq!(
        provider.status(&job).await.unwrap(),
        RemoteStatus::Rejected(Rejection::ExplicitContent)
    );
    assert_eq!(provider.cancel(&job).await, Err(validation(None, "Not queued")));
}

#[tokio::test]
async fn seedance_without_media_makes_a_clip_from_the_prompt() {
    let (provider, stub) = provider(vec![submit_route("/bytedance/seedance-2.5/text-to-video")]).await;

    provider
        .submit(
            &seedance(false, 0, AudioOption::Original),
            &remote(None, Vec::new()),
            Uuid::new_v4(),
        )
        .await
        .unwrap();

    assert_eq!(
        first_body(&stub).await,
        r#"{"aspect_ratio":"9:16","duration":8,"generate_audio":true,"prompt":"a fox in the snow","resolution":"720p"}"#
    );
}

#[tokio::test]
async fn seedance_with_images_only_uses_them_as_references() {
    let (provider, stub) = provider(vec![submit_route("/bytedance/seedance-2.5/reference-to-video")]).await;

    provider
        .submit(
            &seedance(false, 1, AudioOption::Mute),
            &remote(None, vec![character()]),
            Uuid::new_v4(),
        )
        .await
        .unwrap();

    let body = first_body(&stub).await;
    assert!(
        body.contains(r#""image_urls":["https://cdn.example.com/a.jpg"]"#),
        "{body}"
    );
    assert!(body.contains(r#""generate_audio":false"#), "{body}");
    assert!(!body.contains("video_urls"), "{body}");
}

#[tokio::test]
async fn seedance_sends_the_source_first_and_its_frame() {
    let (provider, stub) = provider(vec![(
        "POST",
        "/estimate/bytedance/seedance-2.5/reference-to-video".to_owned(),
        Reply::json_text(r#"{"usd": "6.6"}"#),
    )])
    .await;

    let price = provider
        .server_estimate(
            &seedance(true, 1, AudioOption::Original),
            &remote(Some(video()), vec![character()]),
        )
        .await
        .unwrap();

    assert_eq!(price, Some(usd("6.6", false)));
    assert_eq!(
        first_body(&stub).await,
        r#"{"aspect_ratio":"9:16","duration":12,"generate_audio":false,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"a fox in the snow","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
}

#[tokio::test]
async fn kling_quality_is_the_endpoint() {
    for (resolution, quality) in [
        (Resolution::P480, "std"),
        (Resolution::P720, "std"),
        (Resolution::P1080, "pro"),
    ] {
        let path = format!("/kling-video/v3/motion-control/{quality}");
        let (provider, stub) = provider(vec![submit_route(&path)]).await;
        let mut kling = kling();
        kling.resolution = resolution;
        kling.keeps_video_background = true;

        provider
            .submit(&kling, &remote(Some(video()), vec![character()]), Uuid::new_v4())
            .await
            .unwrap();

        let requests = stub.requests().await;
        let request = requests.first().unwrap();
        assert_eq!(request.url.path(), path);
        assert_eq!(
            body_text(request),
            r#"{"image_url":"https://cdn.example.com/a.jpg","keep_original_sound":"yes","video_url":"https://cdn.example.com/in.mp4"}"#
        );
    }
}

/// Kling keeps the source's sound itself for Original; Mute wants none, and Fade out comes back silent so the faded
/// sound is put on after download.
#[tokio::test]
async fn kling_sends_the_sound_and_the_orientation() {
    for (audio, keeps_sound) in [
        (AudioOption::Original, "yes"),
        (AudioOption::Mute, "no"),
        (AudioOption::FadeOut, "no"),
    ] {
        let (provider, stub) = provider(vec![submit_route("/kling-video/v3/motion-control/pro")]).await;
        let mut kling = kling();
        kling.resolution = Resolution::P1080;
        kling.audio = audio;
        kling.character_orientation = Some(CharacterOrientation::Image);

        provider
            .submit(&kling, &remote(Some(video()), vec![character()]), Uuid::new_v4())
            .await
            .unwrap();

        let body = first_json(&stub).await;
        assert_eq!(body["keep_original_sound"], json!(keeps_sound), "{audio:?}");
        assert_eq!(body["character_orientation"], json!("image"), "{audio:?}");
    }
}

/// With a source, Seedance makes a soundtrack only when asked to; the bitrate goes as picked.
#[tokio::test]
async fn seedance_with_a_source_makes_its_own_sound_only_when_asked() {
    for (audio, generates) in [
        (AudioOption::Original, false),
        (AudioOption::Generated, true),
        (AudioOption::Mute, false),
    ] {
        let (provider, stub) =
            provider(vec![submit_route("/bytedance/seedance-2.5/reference-to-video")]).await;
        let mut job = seedance(true, 0, audio);
        job.bitrate = Some(SeedanceBitrate::Standard);

        provider
            .submit(&job, &remote(Some(video()), Vec::new()), Uuid::new_v4())
            .await
            .unwrap();

        let body = first_json(&stub).await;
        assert_eq!(body["generate_audio"], json!(generates), "{audio:?}");
        assert_eq!(body["bitrate_mode"], json!("standard"), "{audio:?}");
    }
}

#[tokio::test]
async fn seedance_edit_makes_its_own_sound_when_asked() {
    let (provider, stub) = provider(vec![submit_route("/bytedance/seedance-2.5/video-edit")]).await;
    let mut edit = seedance(true, 0, AudioOption::Generated);
    edit.mode = Mode::Edit;

    provider
        .submit(&edit, &remote(Some(video()), Vec::new()), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(first_json(&stub).await["generate_audio"], json!(true));
}

/// The first reference is the start frame and the second the end frame; the frame follows the start image, so no
/// aspect ratio goes with them.
#[tokio::test]
async fn seedance_frames_run_on_image_to_video() {
    let (provider, stub) = provider(vec![submit_route("/bytedance/seedance-2.5/image-to-video")]).await;
    let mut job = seedance(false, 2, AudioOption::Original);
    job.references_are_frames = true;
    let end = RemoteAsset::new(url("https://cdn.example.com/b.jpg"));

    provider
        .submit(&job, &remote(None, vec![character(), end]), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(
        first_body(&stub).await,
        r#"{"duration":8,"end_image_url":"https://cdn.example.com/b.jpg","generate_audio":true,"image_url":"https://cdn.example.com/a.jpg","prompt":"a fox in the snow","resolution":"720p"}"#
    );
    assert_eq!(provider.estimate(&job).await.unwrap(), usd("3.6976", true));
    assert_eq!(
        provider
            .submit(&job, &remote(None, Vec::new()), Uuid::new_v4())
            .await,
        Err(validation(Some("image_url"), "Seedance needs the start frame."))
    );
}

#[tokio::test]
async fn kling_without_an_image_fails_before_sending() {
    let (provider, stub) = provider(Vec::new()).await;

    assert_eq!(
        provider
            .submit(&kling(), &remote(Some(video()), Vec::new()), Uuid::new_v4())
            .await,
        Err(validation(Some("image_url"), "Kling needs a reference image."))
    );
    assert!(stub.requests().await.is_empty());
}

/// Edit sends no length (the output follows the source) and keeps the source's sound; Extend sends the part it adds
/// and has the model make its sound.
#[tokio::test]
async fn seedance_edit_and_extend_send_the_source_as_the_clip() {
    for (mode, path, body) in [
        (
            Mode::Edit,
            "/bytedance/seedance-2.5/video-edit",
            r#"{"generate_audio":false,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"a fox in the snow","resolution":"720p","video_url":"https://cdn.example.com/in.mp4"}"#,
        ),
        (
            Mode::Extend,
            "/bytedance/seedance-2.5/video-extend",
            r#"{"duration":7,"generate_audio":true,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"a fox in the snow","resolution":"720p","video_url":"https://cdn.example.com/in.mp4"}"#,
        ),
    ] {
        let (provider, stub) = provider(vec![submit_route(path)]).await;
        let mut job = seedance(true, 1, AudioOption::Original);
        job.mode = mode;
        job.length = (mode == Mode::Extend).then_some(7.0);

        provider
            .submit(&job, &remote(Some(video()), vec![character()]), Uuid::new_v4())
            .await
            .unwrap();

        let requests = stub.requests().await;
        let request = requests.first().unwrap();
        assert_eq!(request.url.path(), path);
        assert_eq!(body_text(request), body);
    }
}

#[tokio::test]
async fn seedance_edit_without_the_clip_fails_before_sending() {
    let (provider, stub) = provider(Vec::new()).await;
    let mut edit = seedance(true, 0, AudioOption::Original);
    edit.mode = Mode::Edit;

    assert_eq!(
        provider
            .submit(&edit, &remote(None, Vec::new()), Uuid::new_v4())
            .await,
        Err(validation(
            Some("video_url"),
            "Seedance needs the clip to edit or extend."
        ))
    );
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn other_models_have_a_list_price_before_upload() {
    let (provider, _stub) = provider(Vec::new()).await;

    assert_eq!(provider.estimate(&kling()).await.unwrap(), usd("1.512", true));
    assert_eq!(
        provider
            .estimate(&seedance(false, 0, AudioOption::Original))
            .await
            .unwrap(),
        usd("3.6976", true)
    );
}

/// A job with nothing to upload is quoted by `POST /estimate` with the body its submit sends, before anything else
/// happens (issue #113); jobs that differ only in the prompt share the quote.
#[tokio::test]
async fn a_job_without_media_is_quoted_before_upload() {
    let (provider, stub) = provider(vec![(
        "POST",
        "/estimate/bytedance/seedance-2.5/text-to-video".to_owned(),
        Reply::json_text(r#"{"credits": "59.000", "usd": "3.69"}"#),
    )])
    .await;
    let quote = usd("3.69", false);
    let mut reworded = seedance(false, 0, AudioOption::Original);
    "a fox in the rain".clone_into(&mut reworded.prompt);

    assert_eq!(
        provider
            .estimate(&seedance(false, 0, AudioOption::Original))
            .await
            .unwrap(),
        quote
    );
    assert_eq!(provider.estimate(&reworded).await.unwrap(), quote);

    let requests = stub.requests().await;
    assert_eq!(
        requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>(),
        ["/estimate/bytedance/seedance-2.5/text-to-video"]
    );
    assert_eq!(
        body_text(requests.first().unwrap()),
        r#"{"aspect_ratio":"9:16","duration":8,"generate_audio":true,"prompt":"a fox in the snow","resolution":"720p"}"#
    );
}

/// A failed quote leaves the list price, and isn't kept: the next estimate asks again. A job with media, or without
/// the prompt its request needs, isn't quoted before upload.
#[tokio::test]
async fn without_a_quote_the_list_price_stands() {
    let (provider, stub) = provider(vec![(
        "POST",
        "/estimate/bytedance/seedance-2.5/text-to-video".to_owned(),
        Reply::json_text("{}").with_status(503),
    )])
    .await;
    let list_price = usd("3.6976", true);
    let mut unprompted = seedance(false, 0, AudioOption::Original);
    " ".clone_into(&mut unprompted.prompt);

    for job in [
        seedance(false, 0, AudioOption::Original),
        seedance(false, 0, AudioOption::Original),
        unprompted,
        seedance(false, 1, AudioOption::Original),
    ] {
        assert_eq!(provider.estimate(&job).await.unwrap(), list_price);
    }
    assert_eq!(stub.requests().await.len(), 2);
}
