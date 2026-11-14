//! The REST client against a stub host: requests, bodies, status mapping, uploads and returned URLs.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use higgsfield::api::*;
use higgsfield::core::{GenerationStatus, HiggsfieldError, ImageStatus, Moderation};
use rust_decimal::Decimal;
use serde::Serialize;
use url::Url;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::support::stub::{Stub, body_text, dead_url, header, http};

/// Response bodies as documented at docs.higgsfield.ai.
mod fixture {
    use super::*;

    pub const PRESET_ID: &str = "c2143317-f28d-4c3c-a0b8-39bd547e08a7";

    pub const SUBMITTED: &str = r#"{
      "status": "queued",
      "request_id": "req-123",
      "status_url": "https://api.higgsfield.ai/requests/req-123/status",
      "cancel_url": "https://api.higgsfield.ai/requests/req-123/cancel"
    }"#;

    pub const PRESETS: &str = r#"{
      "model": "higgsfield/genjutsu/restyle/v1.0",
      "items": [
        {
          "id": "c2143317-f28d-4c3c-a0b8-39bd547e08a7",
          "name": "Cel-Shaded CG Anime",
          "preview_url": "https://cdn.higgsfield.ai/restyle_presets/259d7ddb-4201-49f7-8a91-cee5bce28715.webp"
        }
      ]
    }"#;

    pub const QUEUE_FULL: &str = r#"{"detail": {"code": "account_queue_full", "message": "Too many queued generations for this account",
      "retryable": true, "limit": 40, "retry_after_seconds": 30}}"#;

    pub const EXPIRED_KEY: &str = r#"{"code": "api_key_expired", "detail": "This API key has expired.", "expires_at": "2026-09-21T12:00:00Z"}"#;

    pub fn status(status: &str, extra: &str) -> String {
        format!(r#"{{"status": "{status}", "request_id": "req-123"{extra}}}"#)
    }

    pub fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    pub fn video_url() -> Url {
        url("https://cdn.example.com/in.mp4")
    }

    pub fn image_urls() -> Vec<Url> {
        vec![
            url("https://cdn.example.com/a.jpg"),
            url("https://cdn.example.com/b.png"),
        ]
    }

    pub fn request(prompt: &str, resolution: &str, preset_id: Option<Uuid>) -> GenerationRequest {
        GenerationRequest {
            video_url: video_url(),
            image_urls: image_urls(),
            prompt: prompt.to_owned(),
            resolution: resolution.to_owned(),
            preset_id,
        }
    }

    pub fn default_request() -> GenerationRequest {
        request("", "480p", None)
    }

    pub fn preset_id() -> Uuid {
        Uuid::parse_str(PRESET_ID).unwrap()
    }
}

use fixture::url;

const IMAGES: &str = r#"["https://cdn.example.com/a.jpg","https://cdn.example.com/b.png"]"#;
const VIDEO: &str = r#""https://cdn.example.com/in.mp4""#;

fn client(stub: &Stub) -> HiggsfieldApiClient {
    HiggsfieldApiClient::new("key-id", "key-secret")
        .with_base_url(stub.base_url.clone())
        .with_http_client(http())
}

fn ok(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string(body)
}

async fn on(stub: &Stub, verb: &str, route: &str, response: ResponseTemplate) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(response)
        .mount(&stub.mock)
        .await;
}

fn source() -> Url {
    url("https://cdn.example.com/in.mp4")
}

fn character() -> Url {
    url("https://cdn.example.com/a.jpg")
}

// MARK: Error mapping through the client

/// A full queue creates nothing, so the submit sent again after the wait carries the same key and can't charge
/// twice.
#[tokio::test]
async fn a_submit_sent_again_after_a_full_queue_keeps_its_idempotency_key() {
    let stub = Stub::https().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_string(fixture::QUEUE_FULL)
                .insert_header("Retry-After", "1"),
        )
        .up_to_n_times(1)
        .mount(&stub.mock)
        .await;
    Mock::given(method("POST"))
        .respond_with(ok(r#"{"status": "queued", "request_id": "req-1"}"#))
        .mount(&stub.mock)
        .await;
    let client = client(&stub);
    let key = Uuid::new_v4();

    let first = client
        .submit(ModelEndpoint::MotionTransfer, &fixture::default_request(), key)
        .await;
    assert_eq!(
        first.unwrap_err(),
        HiggsfieldError::ConcurrencyLimit {
            retry_after: Some(1.0)
        }
    );
    let second = client
        .submit(ModelEndpoint::MotionTransfer, &fixture::default_request(), key)
        .await
        .unwrap();
    assert_eq!(second.request_id, "req-1");
    let keys: Vec<_> = stub
        .requests()
        .await
        .iter()
        .map(|r| header(r, "Idempotency-Key").map(str::to_owned))
        .collect();
    assert_eq!(keys, vec![Some(key.to_string()), Some(key.to_string())]);
}

#[tokio::test]
async fn transport_failure_is_network_error() {
    let client = HiggsfieldApiClient::new("key-id", "key-secret").with_base_url(dead_url(""));
    assert_eq!(
        client.status("req-123", None).await.unwrap_err(),
        HiggsfieldError::Network
    );
}

#[tokio::test]
async fn unreadable_success_body_is_server_error() {
    let stub = Stub::https().await;
    on(&stub, "GET", "/requests/req-123/status", ok("<html>")).await;
    assert_eq!(
        client(&stub).status("req-123", None).await.unwrap_err(),
        HiggsfieldError::Server { status: 200 }
    );
}

const PRESETS_PATH: &str = "/models/higgsfield/genjutsu/restyle/v1.0/presets";

#[tokio::test]
async fn validate_key_accepts_working_key() {
    let stub = Stub::https().await;
    on(&stub, "GET", PRESETS_PATH, ok(fixture::PRESETS)).await;
    assert!(client(&stub).validate_key().await.unwrap());
}

#[tokio::test]
async fn validate_key_rejects_refused_and_expired_keys() {
    for refusal in [
        ResponseTemplate::new(401),
        ResponseTemplate::new(403).set_body_string(fixture::EXPIRED_KEY),
    ] {
        let stub = Stub::https().await;
        on(&stub, "GET", PRESETS_PATH, refusal).await;
        assert!(!client(&stub).validate_key().await.unwrap());
    }
}

#[tokio::test]
async fn validate_key_fails_when_offline() {
    let client = HiggsfieldApiClient::new("key-id", "key-secret").with_base_url(dead_url(""));
    assert_eq!(client.validate_key().await.unwrap_err(), HiggsfieldError::Network);
}

// MARK: Image models

/// Bodies as the model pages at docs.higgsfield.ai/docs/models document them (issue #42).
#[tokio::test]
async fn image_submit_sends_documented_body() {
    async fn check(endpoint: ModelEndpoint, body: &impl Serialize, route: &str, expected: &str) {
        let stub = Stub::https().await;
        on(&stub, "POST", route, ok(fixture::SUBMITTED)).await;
        on(
            &stub,
            "POST",
            &format!("/estimate{route}"),
            ok(r#"{"usd": "0.0128", "credits": "0.2"}"#),
        )
        .await;
        let client = client(&stub);

        assert_eq!(
            client.estimate(endpoint, body).await.unwrap(),
            Decimal::from_str("0.0128").unwrap()
        );
        let job = client.submit(endpoint, body, Uuid::new_v4()).await.unwrap();

        assert_eq!(job.request_id, "req-123");
        let bodies: Vec<_> = stub.requests().await.iter().map(body_text).collect();
        assert_eq!(bodies, vec![expected, expected]);
    }
    let input = url("https://cdn.example.com/a.jpg");
    check(
        ModelEndpoint::SoulV2,
        &SoulImageRequest {
            prompt: "a knight".into(),
            aspect_ratio: "9:16".into(),
            batch_size: 4,
        },
        "/higgsfield-ai/soul/v2/standard",
        r#"{"aspect_ratio":"9:16","batch_size":4,"prompt":"a knight"}"#,
    )
    .await;
    check(
        ModelEndpoint::GrokImage2,
        &GrokImageRequest {
            prompt: "a knight".into(),
            aspect_ratio: "1:1".into(),
            image_urls: vec![],
        },
        "/xai/grok-imagine-image-2.0",
        r#"{"aspect_ratio":"1:1","prompt":"a knight"}"#,
    )
    .await;
    check(
        ModelEndpoint::GrokImage2,
        &GrokImageRequest {
            prompt: "add a cape".into(),
            aspect_ratio: "auto".into(),
            image_urls: vec![input.clone()],
        },
        "/xai/grok-imagine-image-2.0",
        r#"{"aspect_ratio":"auto","image_urls":["https://cdn.example.com/a.jpg"],"prompt":"add a cape"}"#,
    )
    .await;
    check(
        ModelEndpoint::QwenImage3Edit,
        &QwenEditRequest {
            prompt: "add a cape".into(),
            image_urls: vec![input],
            aspect_ratio: "16:9".into(),
        },
        "/alibaba/qwen-image-3/edit",
        r#"{"aspect_ratio":"16:9","image_urls":["https://cdn.example.com/a.jpg"],"prompt":"add a cape"}"#,
    )
    .await;
}

#[tokio::test]
async fn image_status_reads_the_images() {
    let failed = |message: &str| ImageStatus::Failed {
        message: Some(message.to_owned()),
    };
    let cases = [
        (fixture::status("queued", ""), ImageStatus::Queued),
        (fixture::status("in_progress", ""), ImageStatus::InProgress),
        (
            fixture::status(
                "completed",
                r#", "images": [{"url": "https://cdn.example.com/1.png"}, {"url": "https://cdn.example.com/2.png"}]"#,
            ),
            ImageStatus::Completed {
                image_urls: vec![
                    url("https://cdn.example.com/1.png"),
                    url("https://cdn.example.com/2.png"),
                ],
            },
        ),
        (
            fixture::status(
                "completed",
                r#", "images": [{"url": "http://cdn.example.com/1.png"}]"#,
            ),
            failed("Completed without an image URL."),
        ),
        (
            fixture::status("completed", ""),
            failed("Completed without an image URL."),
        ),
        (
            fixture::status("failed", r#", "error": "bad prompt""#),
            failed("bad prompt"),
        ),
        (
            fixture::status("nsfw", ""),
            ImageStatus::Rejected(Moderation::ExplicitContent),
        ),
        (fixture::status("canceled", ""), ImageStatus::Canceled),
    ];
    for (body, expected) in cases {
        let stub = Stub::https().await;
        on(&stub, "GET", "/requests/req-123/status", ok(&body)).await;
        assert_eq!(
            client(&stub).image_status("req-123", None).await.unwrap(),
            expected,
            "{body}"
        );
    }
}

// MARK: Requests

#[tokio::test]
async fn submit_sends_documented_request() {
    let cases = [
        (
            ModelEndpoint::MotionTransfer,
            fixture::default_request(),
            "/higgsfield/genjutsu/motion-transfer/v1.0",
            format!(r#"{{"image_urls":{IMAGES},"prompt":"","resolution":"480p","video_url":{VIDEO}}}"#),
        ),
        (
            ModelEndpoint::ObjectSwap,
            fixture::request("a red car", "1080p", None),
            "/higgsfield/genjutsu/object-swap/v1.0",
            format!(
                r#"{{"image_urls":{IMAGES},"prompt":"a red car","resolution":"1080p","video_url":{VIDEO}}}"#
            ),
        ),
        (
            ModelEndpoint::Restyle,
            fixture::request("", "720p", Some(fixture::preset_id())),
            "/higgsfield/genjutsu/restyle/v1.0",
            format!(
                r#"{{"image_urls":{IMAGES},"preset_id":"c2143317-f28d-4c3c-a0b8-39bd547e08a7","prompt":"","resolution":"720p","video_url":{VIDEO}}}"#
            ),
        ),
    ];
    for (endpoint, generation, route, body) in cases {
        let stub = Stub::https().await;
        on(
            &stub,
            "POST",
            route,
            ok(fixture::SUBMITTED).insert_header("X-Correlation-ID", "corr-1"),
        )
        .await;
        let key = Uuid::new_v4();

        let job = client(&stub).submit(endpoint, &generation, key).await.unwrap();

        assert_eq!(
            job,
            SubmittedRequest {
                request_id: "req-123".into(),
                correlation_id: Some("corr-1".into()),
                status_url: Some(url("https://api.higgsfield.ai/requests/req-123/status")),
                cancel_url: Some(url("https://api.higgsfield.ai/requests/req-123/cancel")),
                initial_status: Some(GenerationStatus::Queued),
            }
        );
        let requests = stub.requests().await;
        let request = &requests[0];
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(request.url.path(), route);
        assert_eq!(header(request, "Authorization"), Some("Key key-id:key-secret"));
        assert_eq!(header(request, "Idempotency-Key"), Some(key.to_string().as_str()));
        assert_eq!(header(request, "Content-Type"), Some("application/json"));
        assert_eq!(body_text(request), body);
        assert!(!body_text(request).contains("null"));
    }
}

async fn submitted_body(endpoint: ModelEndpoint, body: &impl Serialize, route: &str) -> String {
    let stub = Stub::https().await;
    on(&stub, "POST", route, ok(fixture::SUBMITTED)).await;
    client(&stub)
        .submit(endpoint, body, Uuid::new_v4())
        .await
        .unwrap();
    let requests = stub.requests().await;
    assert_eq!(requests[0].url.path(), route);
    body_text(&requests[0])
}

#[tokio::test]
async fn seedance_sends_only_the_fields_it_has() {
    let cases = [
        (
            ModelEndpoint::SeedanceTextToVideo,
            SeedanceRequest {
                prompt: "a fox in the snow".into(),
                image_urls: vec![],
                video_urls: vec![],
                duration: 8,
                resolution: "720p".into(),
                aspect_ratio: Some("9:16".into()),
                generate_audio: true,
                bitrate_mode: None,
            },
            "/bytedance/seedance-2.5/text-to-video",
            r#"{"aspect_ratio":"9:16","duration":8,"generate_audio":true,"prompt":"a fox in the snow","resolution":"720p"}"#,
        ),
        (
            ModelEndpoint::SeedanceReferenceToVideo,
            SeedanceRequest {
                prompt: String::new(),
                image_urls: vec![character()],
                video_urls: vec![source()],
                duration: 12,
                resolution: "480p".into(),
                aspect_ratio: Some("16:9".into()),
                generate_audio: false,
                bitrate_mode: None,
            },
            "/bytedance/seedance-2.5/reference-to-video",
            r#"{"aspect_ratio":"16:9","duration":12,"generate_audio":false,"image_urls":["https://cdn.example.com/a.jpg"],"resolution":"480p","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
        (
            ModelEndpoint::SeedanceReferenceToVideo,
            SeedanceRequest {
                prompt: "dance".into(),
                image_urls: vec![character()],
                video_urls: vec![],
                duration: 5,
                resolution: "1080p".into(),
                aspect_ratio: None,
                generate_audio: true,
                bitrate_mode: None,
            },
            "/bytedance/seedance-2.5/reference-to-video",
            r#"{"duration":5,"generate_audio":true,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"dance","resolution":"1080p"}"#,
        ),
        (
            ModelEndpoint::SeedanceReferenceToVideo,
            SeedanceRequest {
                prompt: String::new(),
                image_urls: vec![],
                video_urls: vec![source()],
                duration: 6,
                resolution: "720p".into(),
                aspect_ratio: None,
                generate_audio: true,
                bitrate_mode: Some("standard".into()),
            },
            "/bytedance/seedance-2.5/reference-to-video",
            r#"{"bitrate_mode":"standard","duration":6,"generate_audio":true,"resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
    ];
    for (endpoint, generation, route, body) in cases {
        let sent = submitted_body(endpoint, &generation, route).await;
        assert_eq!(sent, body);
        assert!(!sent.contains("null"));
    }
}

#[tokio::test]
async fn seedance_edit_and_extend_send_the_source_and_only_the_fields_they_have() {
    let cases = [
        (
            ModelEndpoint::SeedanceVideoEdit,
            SeedanceVideoRequest {
                prompt: "make it night".into(),
                video_url: source(),
                image_urls: vec![],
                duration: None,
                resolution: "720p".into(),
                generate_audio: false,
                bitrate_mode: None,
            },
            "/bytedance/seedance-2.5/video-edit",
            r#"{"generate_audio":false,"prompt":"make it night","resolution":"720p","video_url":"https://cdn.example.com/in.mp4"}"#,
        ),
        (
            ModelEndpoint::SeedanceVideoExtend,
            SeedanceVideoRequest {
                prompt: "she turns around".into(),
                video_url: source(),
                image_urls: vec![character()],
                duration: Some(6),
                resolution: "1080p".into(),
                generate_audio: true,
                bitrate_mode: None,
            },
            "/bytedance/seedance-2.5/video-extend",
            r#"{"duration":6,"generate_audio":true,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"she turns around","resolution":"1080p","video_url":"https://cdn.example.com/in.mp4"}"#,
        ),
    ];
    for (endpoint, generation, route, body) in cases {
        let sent = submitted_body(endpoint, &generation, route).await;
        assert_eq!(sent, body);
        assert!(!sent.contains("null"));
    }
}

#[tokio::test]
async fn kling_sends_the_character_and_the_motion() {
    let cases = [
        (
            ModelEndpoint::KlingMotionControlStandard,
            "/kling-video/v3/motion-control/std",
            "",
        ),
        (
            ModelEndpoint::KlingMotionControlPro,
            "/kling-video/v3/motion-control/pro",
            "walks to the door",
        ),
    ];
    for (endpoint, route, prompt) in cases {
        let stub = Stub::https().await;
        on(&stub, "POST", route, ok(fixture::SUBMITTED)).await;
        on(
            &stub,
            "POST",
            &format!("/estimate{route}"),
            ok(r#"{"usd": "1.512"}"#),
        )
        .await;
        let client = client(&stub);
        let generation = KlingMotionRequest {
            image_url: character(),
            video_url: source(),
            prompt: prompt.into(),
            keep_original_sound: None,
            character_orientation: None,
        };

        assert_eq!(
            client.estimate(endpoint, &generation).await.unwrap(),
            Decimal::from_str("1.512").unwrap()
        );
        client
            .submit(endpoint, &generation, Uuid::new_v4())
            .await
            .unwrap();

        let prompt_field = if prompt.is_empty() {
            String::new()
        } else {
            format!(r#","prompt":"{prompt}""#)
        };
        let body = format!(
            r#"{{"image_url":"https://cdn.example.com/a.jpg"{prompt_field},"video_url":"https://cdn.example.com/in.mp4"}}"#
        );
        let requests = stub.requests().await;
        let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_owned()).collect();
        assert_eq!(paths, vec![format!("/estimate{route}"), route.to_owned()]);
        assert!(requests.iter().all(|r| body_text(r) == body));
    }
}

#[tokio::test]
async fn kling_sends_the_sound_and_orientation_as_the_schema_spells_them() {
    let generation = KlingMotionRequest {
        image_url: character(),
        video_url: source(),
        prompt: String::new(),
        keep_original_sound: Some(OriginalSound::Drop),
        character_orientation: Some(Orientation::Image),
    };
    let sent = submitted_body(
        ModelEndpoint::KlingMotionControlStandard,
        &generation,
        "/kling-video/v3/motion-control/std",
    )
    .await;
    assert_eq!(
        sent,
        r#"{"character_orientation":"image","image_url":"https://cdn.example.com/a.jpg","keep_original_sound":"no","video_url":"https://cdn.example.com/in.mp4"}"#
    );
}

/// The source is the one `video_urls` item and the quality the `mode`; without images, `image_urls` is left out.
#[tokio::test]
async fn kling_o3_sends_the_reference_video_and_the_quality() {
    let cases = [
        (
            KlingVideoReferenceRequest {
                prompt: "she spins".into(),
                video_url: source(),
                image_urls: vec![],
                quality: KlingQuality::Standard,
                duration: 5,
            },
            r#"{"duration":5,"mode":"std","prompt":"she spins","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
        (
            KlingVideoReferenceRequest {
                prompt: "she spins".into(),
                video_url: source(),
                image_urls: vec![character()],
                quality: KlingQuality::Pro,
                duration: 10,
            },
            r#"{"duration":10,"image_urls":["https://cdn.example.com/a.jpg"],"mode":"pro","prompt":"she spins","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
    ];
    for (generation, body) in cases {
        let sent = submitted_body(
            ModelEndpoint::KlingO3VideoReference,
            &generation,
            "/kling-video/o3/video-reference",
        )
        .await;
        assert_eq!(sent, body);
    }
}

/// Cinema Studio's creative controls go in as top-level fields beside the others; `MiniMax` H3 sends its "2K" and no
/// frame, soundtrack or controls.
#[tokio::test]
async fn reference_to_video_sends_only_the_fields_it_has() {
    let cases = [
        (
            ModelEndpoint::CinemaStudio4,
            "/higgsfield/cinema-studio/4.0",
            ReferenceToVideoRequest {
                prompt: "rain".into(),
                video_url: source(),
                image_urls: vec![],
                duration: 6,
                resolution: "480p".into(),
                aspect_ratio: Some("21:9".into()),
                generate_audio: Some(true),
                controls: BTreeMap::from([
                    ("genre".to_owned(), "noir".to_owned()),
                    ("camera_movement".to_owned(), "whip-pan".to_owned()),
                ]),
            },
            r#"{"aspect_ratio":"21:9","camera_movement":"whip-pan","duration":6,"generate_audio":true,"genre":"noir","prompt":"rain","resolution":"480p","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
        (
            ModelEndpoint::MinimaxH3ReferenceToVideo,
            "/minimax/h3/reference-to-video",
            ReferenceToVideoRequest {
                prompt: "rain".into(),
                video_url: source(),
                image_urls: vec![character()],
                duration: 15,
                resolution: "2K".into(),
                aspect_ratio: None,
                generate_audio: None,
                controls: BTreeMap::new(),
            },
            r#"{"duration":15,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"rain","resolution":"2K","video_urls":["https://cdn.example.com/in.mp4"]}"#,
        ),
    ];
    for (endpoint, route, generation, body) in cases {
        assert_eq!(submitted_body(endpoint, &generation, route).await, body);
    }
}

/// The start frame is `image_url`; without an end frame, an empty prompt or a bitrate, those keys are left out.
#[tokio::test]
async fn seedance_frames_go_to_image_to_video() {
    let cases = [
        (
            SeedanceFramesRequest {
                prompt: String::new(),
                image_url: character(),
                end_image_url: None,
                duration: 5,
                resolution: "720p".into(),
                generate_audio: true,
                bitrate_mode: None,
            },
            r#"{"duration":5,"generate_audio":true,"image_url":"https://cdn.example.com/a.jpg","resolution":"720p"}"#,
        ),
        (
            SeedanceFramesRequest {
                prompt: "she turns".into(),
                image_url: character(),
                end_image_url: Some(url("https://cdn.example.com/b.jpg")),
                duration: 8,
                resolution: "1080p".into(),
                generate_audio: false,
                bitrate_mode: Some("high".into()),
            },
            r#"{"bitrate_mode":"high","duration":8,"end_image_url":"https://cdn.example.com/b.jpg","generate_audio":false,"image_url":"https://cdn.example.com/a.jpg","prompt":"she turns","resolution":"1080p"}"#,
        ),
    ];
    for (generation, body) in cases {
        let sent = submitted_body(
            ModelEndpoint::SeedanceImageToVideo,
            &generation,
            "/bytedance/seedance-2.5/image-to-video",
        )
        .await;
        assert_eq!(sent, body);
    }
}

#[tokio::test]
async fn restyle_without_references_sends_empty_array() {
    let stub = Stub::https().await;
    on(
        &stub,
        "POST",
        "/higgsfield/genjutsu/restyle/v1.0",
        ok(fixture::SUBMITTED),
    )
    .await;
    let mut generation = fixture::request("", "480p", Some(fixture::preset_id()));
    generation.image_urls = vec![];

    let job = client(&stub)
        .submit(ModelEndpoint::Restyle, &generation, Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(job.correlation_id, None);
    let body = body_text(&stub.requests().await[0]);
    assert!(body.contains(r#""image_urls":[]"#));
    assert!(body.contains(r#""prompt":"""#));
}

#[tokio::test]
async fn status_and_cancel_use_request_paths() {
    let stub = Stub::https().await;
    on(
        &stub,
        "GET",
        "/requests/req-123/status",
        ok(&fixture::status("queued", "")),
    )
    .await;
    on(
        &stub,
        "POST",
        "/requests/req-123/cancel",
        ResponseTemplate::new(202),
    )
    .await;
    let client = client(&stub);

    assert_eq!(
        client.status("req-123", None).await.unwrap(),
        GenerationStatus::Queued
    );
    client.cancel("req-123", None).await.unwrap();

    let requests = stub.requests().await;
    let methods: Vec<_> = requests.iter().map(|r| r.method.to_string()).collect();
    assert_eq!(methods, ["GET", "POST"]);
    assert!(
        requests
            .iter()
            .all(|r| header(r, "Authorization") == Some("Key key-id:key-secret"))
    );
}

fn temp_video() -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
    std::fs::write(file.path(), "fake video").unwrap();
    file
}

#[tokio::test]
async fn upload_puts_file_without_authorization() {
    let file = temp_video();
    let stub = Stub::https().await;
    let upload_url = format!("{}bucket/in.mp4?X-Signature=abc", stub.base_url);
    let target = format!(
        r#"{{"upload_url": "{upload_url}", "public_url": "https://cdn.example.com/in.mp4",
         "content_type": "video/mp4",
         "upload_headers": {{"Content-Type": "video/mp4", "x-amz-tagging": "retention=temporary"}}}}"#
    );
    on(&stub, "POST", "/files/generate-upload-url", ok(&target)).await;
    on(&stub, "PUT", "/bucket/in.mp4", ResponseTemplate::new(200)).await;
    let client = client(&stub);

    let target = client.upload_target("video/mp4").await.unwrap();
    client
        .put(
            file.path(),
            "video/mp4",
            &target.upload_url,
            &target.upload_headers.clone().unwrap_or_default(),
        )
        .await
        .unwrap();

    assert_eq!(target.public_url, url("https://cdn.example.com/in.mp4"));
    let requests = stub.requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method.as_str(), "POST");
    assert_eq!(
        header(&requests[0], "Authorization"),
        Some("Key key-id:key-secret")
    );
    assert_eq!(body_text(&requests[0]), r#"{"content_type":"video/mp4"}"#);
    assert_eq!(requests[1].method.as_str(), "PUT");
    assert_eq!(
        format!(
            "{}?{}",
            requests[1].url.path(),
            requests[1].url.query().unwrap_or_default()
        ),
        "/bucket/in.mp4?X-Signature=abc"
    );
    assert_eq!(header(&requests[1], "Authorization"), None);
    assert_eq!(header(&requests[1], "Content-Type"), Some("video/mp4"));
    assert_eq!(header(&requests[1], "x-amz-tagging"), Some("retention=temporary"));
    assert_eq!(body_text(&requests[1]), "fake video");
}

#[tokio::test]
async fn storage_refusal_is_not_a_key_or_balance_error() {
    let file = temp_video();
    let stub = Stub::https().await;
    on(
        &stub,
        "PUT",
        "/bucket/in.mp4",
        ResponseTemplate::new(403).set_body_string("<Error>Request has expired</Error>"),
    )
    .await;

    let result = client(&stub)
        .put(
            file.path(),
            "video/mp4",
            &stub.url("/bucket/in.mp4"),
            &HashMap::new(),
        )
        .await;

    assert_eq!(result.unwrap_err(), HiggsfieldError::Server { status: 403 });
}

#[tokio::test]
async fn upload_url_that_is_not_https_is_never_opened() {
    let file = temp_video();
    let stub = Stub::http().await;
    on(&stub, "PUT", "/bucket/in.mp4", ResponseTemplate::new(200)).await;
    for target in [stub.url("/bucket/in.mp4"), url("file:///tmp/in.mp4")] {
        let result = client(&stub)
            .put(file.path(), "video/mp4", &target, &HashMap::new())
            .await;
        assert_eq!(result.unwrap_err(), HiggsfieldError::Server { status: 200 });
    }
    assert!(stub.requests().await.is_empty());
}

#[tokio::test]
async fn presets_decode_documented_catalog() {
    let stub = Stub::https().await;
    on(&stub, "GET", PRESETS_PATH, ok(fixture::PRESETS)).await;

    let presets = client(&stub).presets().await.unwrap();

    assert_eq!(
        presets,
        vec![Preset {
            id: fixture::preset_id(),
            name: "Cel-Shaded CG Anime".into(),
            preview_url: url(
                "https://cdn.higgsfield.ai/restyle_presets/259d7ddb-4201-49f7-8a91-cee5bce28715.webp"
            ),
        }]
    );
    assert_eq!(
        header(&stub.requests().await[0], "Authorization"),
        Some("Key key-id:key-secret")
    );
}

// MARK: Estimates and returned URLs

#[tokio::test]
async fn estimate_sends_the_submit_body_and_reads_the_usd_string() {
    let stub = Stub::https().await;
    let route = "/estimate/higgsfield/genjutsu/motion-transfer/v1.0";
    on(
        &stub,
        "POST",
        route,
        ok(r#"{"credits": "61.000", "usd": "3.816"}"#),
    )
    .await;

    let price = client(&stub)
        .estimate(ModelEndpoint::MotionTransfer, &fixture::default_request())
        .await
        .unwrap();

    assert_eq!(price, Decimal::from_str("3.816").unwrap());
    let request = &stub.requests().await[0];
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(header(request, "Authorization"), Some("Key key-id:key-secret"));
    assert_eq!(header(request, "Idempotency-Key"), None);
    assert_eq!(
        body_text(request),
        format!(
            r#"{{"image_urls":{IMAGES},"prompt":"","resolution":"480p","video_url":"https://cdn.example.com/in.mp4"}}"#
        )
    );
}

#[tokio::test]
async fn unreadable_estimate_is_a_server_error() {
    let stub = Stub::https().await;
    on(
        &stub,
        "POST",
        "/estimate/higgsfield/genjutsu/motion-transfer/v1.0",
        ok(r#"{"usd": "n/a"}"#),
    )
    .await;
    let result = client(&stub)
        .estimate(ModelEndpoint::MotionTransfer, &fixture::default_request())
        .await;
    assert_eq!(result.unwrap_err(), HiggsfieldError::Server { status: 200 });
}

#[tokio::test]
async fn submit_status_maps_like_a_poll() {
    let cases = [
        (fixture::status("queued", ""), Some(GenerationStatus::Queued)),
        (
            fixture::status(
                "completed",
                r#", "video": {"url": "https://cdn.example.com/out.mp4"}"#,
            ),
            Some(GenerationStatus::Completed {
                video_url: url("https://cdn.example.com/out.mp4"),
            }),
        ),
        // Paid for: the first poll fetches the result instead of failing it here.
        (fixture::status("completed", ""), None),
        (r#"{"request_id": "req-123"}"#.to_owned(), None),
    ];
    for (body, expected) in cases {
        let stub = Stub::https().await;
        on(
            &stub,
            "POST",
            "/higgsfield/genjutsu/motion-transfer/v1.0",
            ok(&body),
        )
        .await;
        let job = client(&stub)
            .submit(
                ModelEndpoint::MotionTransfer,
                &fixture::default_request(),
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert_eq!(job.initial_status, expected, "{body}");
    }
}

#[tokio::test]
async fn returned_urls_on_the_api_host_are_used_for_status_and_cancel() {
    let stub = Stub::https().await;
    on(
        &stub,
        "GET",
        "/v2/requests/req-9/status",
        ok(&fixture::status("queued", "")),
    )
    .await;
    on(
        &stub,
        "POST",
        "/v2/requests/req-9/cancel",
        ResponseTemplate::new(202),
    )
    .await;
    let status_url = stub.url("/v2/requests/req-9/status");
    let cancel_url = stub.url("/v2/requests/req-9/cancel");
    let client = client(&stub);

    assert_eq!(
        client.status("req-9", Some(&status_url)).await.unwrap(),
        GenerationStatus::Queued
    );
    client.cancel("req-9", Some(&cancel_url)).await.unwrap();

    let requests = stub.requests().await;
    let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_owned()).collect();
    assert_eq!(paths, ["/v2/requests/req-9/status", "/v2/requests/req-9/cancel"]);
    assert!(
        requests
            .iter()
            .all(|r| header(r, "Authorization") == Some("Key key-id:key-secret"))
    );
}

#[tokio::test]
async fn returned_urls_elsewhere_never_get_the_key() {
    let stub = Stub::https().await;
    let elsewhere = Stub::https().await;
    on(
        &stub,
        "GET",
        "/requests/req-9/status",
        ok(&fixture::status("queued", "")),
    )
    .await;
    on(
        &stub,
        "POST",
        "/requests/req-9/cancel",
        ResponseTemplate::new(202),
    )
    .await;
    let status_url = elsewhere.url("/requests/req-9/status");
    let cancel_url = elsewhere.url("/requests/req-9/cancel");
    let client = client(&stub);

    assert_eq!(
        client.status("req-9", Some(&status_url)).await.unwrap(),
        GenerationStatus::Queued
    );
    client.cancel("req-9", Some(&cancel_url)).await.unwrap();

    assert!(elsewhere.requests().await.is_empty());
    let paths: Vec<_> = stub
        .requests()
        .await
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(paths, ["/requests/req-9/status", "/requests/req-9/cancel"]);
}

#[test]
fn only_https_on_the_api_host_is_trusted_and_a_missing_url_falls_back() {
    let untrusted = [
        "http://api.higgsfield.ai/requests/r/status",
        "https://api.higgsfield.ai.evil.example/requests/r/status",
        "https://evil.example/requests/r/status",
        "https://api.higgsfield.ai:8443/requests/r/status",
        "ftp://api.higgsfield.ai/requests/r/status",
    ];
    let client = HiggsfieldApiClient::new("k", "s");
    let fallback = url("https://api.higgsfield.ai/requests/r/status");
    for returned in untrusted {
        assert_eq!(
            client.trusted_url(Some(&url(returned)), "requests/r/status"),
            fallback,
            "{returned}"
        );
    }
    // The API host in any case is kept; a missing URL falls back.
    let returned = url("https://API.higgsfield.ai/v2/requests/r/status");
    assert_eq!(client.trusted_url(Some(&returned), "requests/r/status"), returned);
    assert_eq!(client.trusted_url(None, "requests/r/status"), fallback);
}

#[tokio::test]
async fn status_maps_to_generation_status() {
    let cases = [
        (fixture::status("queued", ""), GenerationStatus::Queued),
        (fixture::status("in_progress", ""), GenerationStatus::InProgress),
        (
            fixture::status(
                "completed",
                r#", "video": {"url": "https://example.com/output.mp4"}"#,
            ),
            GenerationStatus::Completed {
                video_url: url("https://example.com/output.mp4"),
            },
        ),
        (
            fixture::status("completed", ""),
            GenerationStatus::Failed {
                message: Some("Completed without a video URL.".into()),
            },
        ),
        (
            fixture::status("failed", r#", "error": "Could not download video_url""#),
            GenerationStatus::Failed {
                message: Some("Could not download video_url".into()),
            },
        ),
        (
            fixture::status("failed", ""),
            GenerationStatus::Failed { message: None },
        ),
        (
            fixture::status("nsfw", ""),
            GenerationStatus::Rejected(Moderation::ExplicitContent),
        ),
        (fixture::status("canceled", ""), GenerationStatus::Canceled),
        (fixture::status("upscaling", ""), GenerationStatus::InProgress),
    ];
    for (body, expected) in cases {
        let stub = Stub::https().await;
        on(&stub, "GET", "/requests/req-123/status", ok(&body)).await;
        assert_eq!(
            client(&stub).status("req-123", None).await.unwrap(),
            expected,
            "{body}"
        );
    }
}

#[test]
fn the_client_prints_without_its_secret() {
    let client = HiggsfieldApiClient::new("key-id", "key-secret");
    let printed = format!("{client:?}");
    assert!(printed.contains("key-id"));
    assert!(!printed.contains("key-secret"));
}
