//! Port of `Kling4FlashTests.swift`: Kling 4.0 Flash runs Kling 4.0's code with its own catalog entry, REST's four
//! `v4.0-flash` endpoints from their model pages (2026-11-11) and `kling_4_0_flash`'s `mode`s against its
//! `models_explore get` in the catalog snapshot of 2026-11-10. It renders at 720p only, for 3–20 s, from a start frame
//! without an end frame.

use higgsfield::api::ModelEndpoint;
use higgsfield::mcp::account::VideoRequest;
use jutsu_core::{
    AspectRatio, GenerationProvider, JobSpec, Mode, Pricing, Resolution, ValidationIssue, VideoModel,
};
use serde_json::json;
use uuid::Uuid;

use super::support::agent_c::kling4::{self, Clip, OnSource};
use super::support::agent_c::{
    account_provider, api_body, assert_json, assert_none, assert_params_keep_to_snapshot, check, column,
    estimated, generate_params, inputs, no_inputs, quote, submitted, usd,
};

fn flash() -> VideoModel {
    VideoModel::from_raw("kling40Flash").unwrap()
}

/// Kling 4.0's job of the same kind (`Network.Kling4`), on Flash at its one resolution.
fn on_flash(mut spec: JobSpec) -> JobSpec {
    spec.model = flash();
    spec.resolution = Resolution::P720;
    spec
}

fn clip(images: usize, seconds: f64) -> JobSpec {
    on_flash(
        Clip {
            images,
            seconds,
            aspect_ratio: Some(AspectRatio::Portrait),
            ..Clip::default()
        }
        .spec(),
    )
}

fn frames(count: usize) -> JobSpec {
    on_flash(kling4::frames(count, ""))
}

fn on_source(mode: Mode, seconds: f64) -> JobSpec {
    on_flash(
        OnSource {
            mode,
            seconds,
            ..OnSource::default()
        }
        .spec(),
    )
}

// MARK: REST

/// Each kind of job goes to Flash's own endpoint with Kling 4.0's body: 720p, the length inside 3–20 s, and from
/// frames the start alone.
#[test]
fn each_mode_has_its_flash_endpoint() {
    let none = no_inputs();
    let text = api_body(&clip(0, 12.0), &none);
    assert_eq!(text.0, ModelEndpoint::Kling4FlashTextToVideo);
    assert_eq!(
        text.1,
        r#"{"aspect_ratio":"9:16","duration":12,"prompt":"a fox in the snow","resolution":"720p"}"#
    );
    assert!(api_body(&clip(0, 1.0), &none).1.contains(r#""duration":3,"#));
    assert!(api_body(&clip(0, 25.0), &none).1.contains(r#""duration":20,"#));

    let framed = api_body(&frames(1), &inputs(None, vec![kling4::start()]));
    assert_eq!(framed.0, ModelEndpoint::Kling4FlashImageToVideo);
    assert_eq!(
        framed.1,
        r#"{"duration":5,"image_url":"https://cdn.example.com/a.jpg","resolution":"720p"}"#
    );
    // Its schema has no end frame and refuses unknown fields, so a second image is never sent as one.
    let two = api_body(&frames(2), &inputs(None, vec![kling4::start(), kling4::end()]));
    assert!(!two.1.contains("end_image_url"));

    let with_video = inputs(Some(kling4::video()), vec![kling4::start()]);
    let reference = api_body(&on_source(Mode::MotionTransfer, 10.0), &with_video);
    assert_eq!(reference.0, ModelEndpoint::Kling4FlashReferenceToVideo);
    assert_eq!(
        reference.1,
        r#"{"aspect_ratio":"adaptive","duration":10,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_urls":["https://cdn.example.com/in.mp4"]}"#
    );
    let edit = api_body(&on_source(Mode::Edit, 30.0), &with_video);
    assert_eq!(edit.0, ModelEndpoint::Kling4FlashVideoEdit);
    assert_eq!(
        edit.1,
        r#"{"aspect_ratio":"adaptive","duration":20,"image_urls":["https://cdn.example.com/a.jpg"],"prompt":"make it rain","resolution":"720p","video_url":"https://cdn.example.com/in.mp4"}"#
    );
}

// MARK: Account

/// `kling_4_0_flash`'s four `mode`s, each keeping to its `models_explore get`: 720p, 3–20 s, the frame as
/// `start_image` alone, the source and images as references.
#[tokio::test]
async fn the_account_sends_each_mode() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    let jobs = [
        (clip(0, 12.0), no_inputs()),
        (frames(1), inputs(None, vec![kling4::start()])),
        (
            on_source(Mode::MotionTransfer, 10.0),
            inputs(Some(kling4::video()), vec![kling4::start()]),
        ),
        (
            on_source(Mode::Edit, 40.0),
            inputs(Some(kling4::video()), vec![kling4::start()]),
        ),
    ];
    for (spec, inputs) in &jobs {
        provider.submit(spec, inputs, Uuid::new_v4()).await.unwrap();
    }
    let params = generate_params(&stub).await;
    assert_eq!(params.len(), jobs.len());
    assert_params_keep_to_snapshot(&params);
    assert!(
        params
            .iter()
            .all(|params| params["model"] == "kling_4_0_flash" && params["resolution"] == "720p")
    );
    assert_json(
        &column(&params, "mode"),
        &json!([
            "text-to-video",
            "image-to-video",
            "reference-to-video",
            "video-edit"
        ]),
    );
    assert_json(&params[0]["duration"], &json!(12));
    assert_eq!(params[0]["aspect_ratio"], "9:16");
    assert_json(
        &params[1]["medias"],
        &json!([{"role": "start_image", "value": "image-1"}]),
    );
    assert_eq!(params[2]["aspect_ratio"], "auto");
    assert_json(&params[3]["duration"], &json!(20));
}

// MARK: Validation

/// One start frame, no end frame; up to 20 s; ten images.
#[test]
fn validation_keeps_to_the_schemas() {
    assert_none(&check(&clip(0, 12.0)));
    assert_none(&check(&frames(1)));
    assert!(check(&frames(2)).contains(&ValidationIssue::TooManyReferences { maximum: 1 }));
    assert!(check(&clip(11, 12.0)).contains(&ValidationIssue::TooManyReferences { maximum: 10 }));
    assert!(check(&on_source(Mode::Edit, 21.0)).contains(&ValidationIssue::DurationTooLong(21.0)));
    assert_none(&check(&on_source(Mode::MotionTransfer, 20.0)));
}

// MARK: Prices

/// The REST list price: $0.094 a second without an input video; with one, $0.063 on the output's seconds plus as
/// many of the input's, which are the selection's.
#[test]
fn rest_list_prices() {
    let pricing = Pricing::bundled().unwrap();
    for spec in [clip(0, 10.0), clip(1, 10.0)] {
        assert_eq!(pricing.cost(&spec).unwrap(), usd("0.94", true));
    }
    let mut framed = frames(1);
    framed.length = Some(10.0);
    assert_eq!(pricing.cost(&framed).unwrap(), usd("0.94", true));
    for spec in [on_source(Mode::MotionTransfer, 10.0), on_source(Mode::Edit, 10.0)] {
        assert_eq!(pricing.cost(&spec).unwrap(), usd("1.26", true));
    }
}

/// On the account, until a quote: 3 credits a second, 4 with a source video, derived from REST's prices against
/// Kling 4.0's (no credit price is published). A prompt alone is quoted before upload.
#[test]
fn the_account_estimates_until_a_quote() {
    assert_eq!(estimated(&clip(0, 10.0)), Some(30));
    assert_eq!(estimated(&frames(1)), Some(15));
    assert_eq!(estimated(&on_source(Mode::MotionTransfer, 10.0)), Some(40));
    assert_eq!(estimated(&on_source(Mode::Edit, 12.0)), Some(48));
    let model = "kling_4_0_flash";
    let request = quote(&clip(0, 12.0), model).unwrap();
    assert!(
        request.mode.as_deref() == Some(VideoRequest::KLING4_TEXT_TO_VIDEO)
            && request.resolution.as_deref() == Some("720p")
    );
    assert!(quote(&on_source(Mode::MotionTransfer, 10.0), model).is_none());
}
