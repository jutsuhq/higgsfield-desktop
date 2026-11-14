//! Port of `MCPProviderQuoteTests.swift`: the price before upload (issue #113). A model sent a length is quoted
//! with `get_cost` and no media, once per what the price depends on; a model billed by its source gets the
//! catalog's estimate until it is uploaded.

use std::sync::Arc;

use jutsu_core::{
    AspectRatio, AudioOption, ClosedRange, GenerationProvider, JobSpec, LocalImage, Mode,
    ModelConstraintTable, PixelSize, Resolution, SeedanceBitrate, SourceClip, VideoModel,
};
use serde_json::{Value, json};

use super::support::account::{calls, schema_violations};
use super::support::agent_b::{
    Answers, assert_same, credits, file_url, generate_params, inputs, provider, provider_tools,
};
use super::support::catalog;
use crate::account_model;

fn quote(_: &str, _: &Value) -> Value {
    json!({"cost": {"credits": 35, "credits_exact": 35.5}})
}

/// A model `models.json` lists; its absence fails the catalog tests.
fn model(id: &str) -> VideoModel {
    VideoModel::from_raw(id).unwrap_or(VideoModel::GENJUTSU)
}

fn spec(model: VideoModel, mode: Mode, seconds: f64, resolution: Resolution) -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        30.0,
        PixelSize {
            width: 1280,
            height: 720,
        },
        ClosedRange::new(0.0, seconds),
    );
    let mut spec = JobSpec::new(mode, model, Some(source), resolution);
    spec.references = vec![LocalImage::new(file_url("/tmp/a.jpg"))];
    spec.prompt = "make it rain".into();
    spec
}

/// `spec` at its defaults: Motion Transfer, 5 s, 720p.
fn motion(model: VideoModel) -> JobSpec {
    spec(model, Mode::MotionTransfer, 5.0, Resolution::P720)
}

/// Seedance from a prompt alone (`t2v`), Seedance's Extend, Kling 3.0 (which needs `duration`), Wan 3.0, Wan 3.0
/// Prime and Cinema Studio 4.0.
fn per_second() -> Vec<JobSpec> {
    let mut prompt = motion(VideoModel::SEEDANCE25);
    prompt.source = None;
    prompt.references = Vec::new();
    prompt.length = Some(8.0);
    prompt.aspect_ratio = Some(AspectRatio::Portrait);
    let mut specs = vec![
        prompt,
        spec(VideoModel::SEEDANCE25, Mode::Extend, 5.0, Resolution::P720),
    ];
    specs.push(motion(VideoModel::KLING_MOTION_CONTROL));
    specs.extend(["wan30Reference", "wan30PrimeReference", "cinemaStudio4"].map(|id| motion(model(id))));
    specs
}

#[tokio::test]
async fn a_model_sent_a_length_is_quoted_before_upload() {
    for spec in per_second() {
        let label = format!("{:?} {:?}", spec.model, spec.mode);
        let (provider, stub) = provider(provider_tools(), quote).await;
        assert_eq!(
            provider.estimate(&spec).await.unwrap(),
            credits("35.5", false),
            "{label}"
        );

        let quotes = generate_params(&stub).await;
        assert_eq!(quotes.len(), 1, "{label}");
        let sent = &quotes[0];
        assert_eq!(sent["get_cost"], true, "{label}");
        assert_eq!(sent["use_unlim"], false, "{label}");
        assert!(sent["duration"].is_number(), "{label}");
        // Nothing is uploaded yet, and the price doesn't depend on the prompt or the variants.
        assert!(
            sent.get("medias").is_none() && sent.get("prompt").is_none() && sent.get("count").is_none(),
            "{label}"
        );
        assert!(schema_violations(&stub).await.is_empty(), "{label}");
        let known = "kling3_0_motion_control: duration isn't a parameter";
        let violations: Vec<String> = catalog::violations_of_params(sent)
            .into_iter()
            .filter(|violation| violation != known)
            .collect();
        assert!(violations.is_empty(), "{label}: {violations:?}");
    }
}

/// Genjutsu, Seedance's Edit and Kling O3 Edit follow the source, which `get_cost` needs in `medias`: before upload
/// they show the estimate, and the quote with the uploaded source is exact.
#[tokio::test]
async fn a_model_billed_by_its_source_is_quoted_after_upload() {
    let specs = [
        spec(VideoModel::GENJUTSU, Mode::MotionTransfer, 12.0, Resolution::P720),
        spec(VideoModel::GENJUTSU, Mode::ObjectSwap, 12.0, Resolution::P720),
        spec(VideoModel::SEEDANCE25, Mode::Edit, 12.0, Resolution::P720),
        spec(model("klingO3VideoEdit"), Mode::Edit, 12.0, Resolution::P720),
    ];
    for spec in specs {
        let label = format!("{:?} {:?}", spec.model, spec.mode);
        let (provider, stub) = provider(provider_tools(), quote).await;
        let estimate = provider.estimate(&spec).await.unwrap();
        assert!(estimate.is_estimate() && !estimate.is_unknown_price(), "{label}");
        assert!(calls(&stub).await.is_empty(), "{label}");

        assert_eq!(
            provider.server_estimate(&spec, &inputs()).await.unwrap(),
            Some(credits("35.5", false)),
            "{label}"
        );
        let sent = generate_params(&stub).await.remove(0);
        assert_eq!(sent["get_cost"], true, "{label}");
        assert_eq!(sent["medias"][0]["value"], "video-1", "{label}");
        assert!(schema_violations(&stub).await.is_empty(), "{label}");
    }
}

/// The estimates of the models billed by their source match the live quotes for a 12 s clip (2026-10-15).
#[test]
fn the_estimate_of_a_source_billed_model_is_the_live_rate() {
    let cases = [
        (
            spec(VideoModel::GENJUTSU, Mode::MotionTransfer, 12.0, Resolution::P480),
            36,
        ),
        (
            spec(
                VideoModel::GENJUTSU,
                Mode::MotionTransfer,
                12.0,
                Resolution::P1080,
            ),
            240,
        ),
        (
            spec(VideoModel::GENJUTSU, Mode::ObjectSwap, 12.0, Resolution::P480),
            39,
        ),
        (
            spec(VideoModel::GENJUTSU, Mode::ObjectSwap, 12.0, Resolution::P720),
            91,
        ),
        (
            spec(VideoModel::GENJUTSU, Mode::ObjectSwap, 12.0, Resolution::P1080),
            260,
        ),
        (
            spec(VideoModel::SEEDANCE25, Mode::Edit, 12.0, Resolution::P480),
            43,
        ),
        (
            spec(VideoModel::SEEDANCE25, Mode::Edit, 12.0, Resolution::P720),
            91,
        ),
        (
            spec(VideoModel::SEEDANCE25, Mode::Edit, 12.0, Resolution::P1080),
            241,
        ),
    ];
    for (spec, expected) in cases {
        assert_eq!(
            account_model::estimated_credits(&spec, &ModelConstraintTable::built_in()),
            Some(expected),
            "{:?} {:?} {:?}",
            spec.model,
            spec.mode,
            spec.resolution
        );
    }
}

/// A quote is asked once per model, mode, resolution, length and soundtrack, even while one is out: another prompt,
/// frame or bitrate reads the same answer, another length asks again.
#[tokio::test]
async fn quotes_are_kept_by_what_the_price_depends_on() {
    let (provider, stub) = provider(provider_tools(), quote).await;
    let mut base = motion(VideoModel::SEEDANCE25);
    base.source = None;
    base.references = Vec::new();
    base.length = Some(6.0);
    let mut other_prompt = base.clone();
    other_prompt.prompt = "a fox in the snow".into();
    let mut other_frame = base.clone();
    other_frame.aspect_ratio = Some(AspectRatio::Portrait);
    let mut other_bitrate = base.clone();
    other_bitrate.bitrate = Some(SeedanceBitrate::High);

    let specs = [base.clone(), other_prompt, other_frame, other_bitrate];
    let costs = futures::future::join_all(specs.iter().map(|spec| provider.estimate(spec))).await;
    assert_eq!(costs.len(), 4);
    assert!(
        costs
            .into_iter()
            .all(|cost| cost.unwrap() == credits("35.5", false))
    );
    assert_eq!(generate_params(&stub).await.len(), 1);

    let mut longer = base.clone();
    longer.length = Some(7.0);
    let mut louder = base.clone();
    louder.audio = AudioOption::Mute;
    provider.estimate(&longer).await.unwrap();
    provider.estimate(&louder).await.unwrap();
    provider.estimate(&base).await.unwrap();
    let quotes = generate_params(&stub).await;
    let durations: Vec<Value> = quotes.iter().map(|quote| quote["duration"].clone()).collect();
    assert_same(&Value::Array(durations), &json!([6, 7, 6]));
    let audio: Vec<Value> = quotes
        .iter()
        .map(|quote| quote["generate_audio"].clone())
        .collect();
    assert_eq!(audio, [json!(true), json!(true), json!(false)]);
}

/// A failed quote shows the estimate and isn't kept: the next estimate asks again.
#[tokio::test]
async fn a_failed_quote_falls_back_to_the_estimate_and_is_asked_again() {
    let answers = Arc::new(Answers::new(vec![json!({}), json!({"cost": {"credits": 21}})]));
    let (provider, stub) = provider(provider_tools(), move |_, _| answers.next()).await;
    let spec = spec(
        VideoModel::KLING_MOTION_CONTROL,
        Mode::MotionTransfer,
        12.0,
        Resolution::P720,
    );
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("21", true));
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("21", false));
    assert_eq!(generate_params(&stub).await.len(), 2);
}

/// `count` isn't in the quote (live: 2 quoted the same as 1), so the price is one variant's times the count.
#[tokio::test]
async fn variants_multiply_the_quote() {
    let (provider, stub) = provider(provider_tools(), quote).await;
    let mut spec = motion(model("wan30Reference"));
    spec.variants = 3;
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("106.5", false));
    assert!(generate_params(&stub).await[0].get("count").is_none());
}

/// The quote before upload is the price the job is queued with and that a cap checks: the engine reads it with
/// `estimate`, and quotes the uploaded files again right before submit.
#[tokio::test]
async fn the_quote_before_upload_is_the_jobs_price_until_the_upload_is_quoted() {
    let (provider, stub) = provider(provider_tools(), |_, arguments| {
        if arguments["params"].get("medias").is_none() {
            json!({"cost": {"credits": 35}})
        } else {
            json!({"cost": {"credits": 35, "credits_exact": 35.25}})
        }
    })
    .await;
    let spec = motion(VideoModel::SEEDANCE25);
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("35", false));
    assert_eq!(
        provider.server_estimate(&spec, &inputs()).await.unwrap(),
        Some(credits("35.25", false))
    );
    let quotes = generate_params(&stub).await;
    let without_media: Vec<bool> = quotes.iter().map(|quote| quote.get("medias").is_none()).collect();
    assert_eq!(without_media, [true, false]);
    assert!(quotes.iter().all(|quote| quote["get_cost"] == true));
}
