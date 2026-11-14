//! Port of `GenjutsuLiteTests.swift`: Genjutsu Lite (issue #15), Genjutsu's Motion Transfer and Object Swap on Wan
//! 3.0, as `generate_video` models of their own. With an API key, Motion Transfer alone on its REST endpoint.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use higgsfield::mcp::account::ModelLimits;
use jutsu_core::{
    ClosedRange, GenerationProvider, JobSpec, Mode, ModelConstraintEntry, ModelConstraintTable,
    ModelConstraints, ModelConstraintsSource, PixelSize, PortError, RemoteInputs, Resolution, Validation,
    ValidationIssue, VideoModel,
};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::ResponseTemplate;

use super::support::account::{calls, full_set, schema_violations};
use super::support::agent_c::{
    account_provider, assert_json, assert_none, asset, check, credits, file, images, source, submitted, url,
    usd,
};
use super::support::catalog;
use super::support::mcp::{McpServer, tool};
use super::support::provider::{make_api_provider, make_provider};
use super::support::stub::{Stub, body_json};
use crate::account_model;
use crate::api_model::ApiModel;
use crate::history_mapping;

fn lite() -> VideoModel {
    VideoModel::from_raw("genjutsuLite").unwrap_or(VideoModel::GENJUTSU)
}

fn spec_of(mode: Mode, seconds: f64, resolution: Resolution) -> JobSpec {
    let clip = source("/tmp/in.mov", 30.0, PixelSize::new(1280, 720), seconds);
    let mut spec = JobSpec::new(mode, lite(), Some(clip), resolution);
    spec.references = images(1, "/tmp/a.jpg");
    spec.prompt = "make it rain".to_owned();
    spec
}

fn spec(mode: Mode) -> JobSpec {
    spec_of(mode, 12.0, Resolution::P720)
}

fn medias() -> Value {
    json!([
        {"role": "video_references", "value": "video-1"},
        {"role": "image_references", "value": "image-1"},
    ])
}

/// `Provider.inputs`.
fn account_inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(asset(file("/in.mp4").as_str(), "video-1")),
        images: vec![asset(file("/a.jpg").as_str(), "image-1")],
        audio: None,
    }
}

fn rest_inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(jutsu_core::RemoteAsset::new(url(
            "https://cdn.example.com/in.mp4",
        ))),
        images: vec![jutsu_core::RemoteAsset::new(url("https://cdn.example.com/a.jpg"))],
        audio: None,
    }
}

/// Each mode sends Genjutsu's request to its own model: the media and the resolution, nothing else.
#[tokio::test]
async fn each_mode_sends_genjutsus_request() {
    for (mode, model, resolution) in [
        (
            Mode::MotionTransfer,
            "hf_mult_motion_control_flash",
            Resolution::P480,
        ),
        (
            Mode::ObjectSwap,
            "hf_mult_replace_object_flash",
            Resolution::P1080,
        ),
    ] {
        let (provider, stub) = account_provider(submitted("job-1")).await;
        provider
            .submit(
                &spec_of(mode, 12.0, resolution),
                &account_inputs(),
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let params = calls(&stub).await[0]["arguments"]["params"].clone();
        assert_json(
            &params,
            &json!({
                "model": model, "medias": medias(), "prompt": "make it rain",
                "resolution": resolution.raw_value(), "use_unlim": false,
            }),
        );
        assert_none(&schema_violations(&stub).await);
        assert_none(&catalog::violations_of_params(&params));
    }
}

/// Object Swap renders at 720p and 1080p only (`hf_mult_replace_object_flash`, snapshot of 2026-10-28), so it never
/// sends 480p: a job asking for it is made at 720p, and so is one stored at 480p before, which the Queue, the
/// Library and the estimate then show at 720p too. Motion Transfer keeps 480p.
#[tokio::test]
async fn object_swap_never_sends480p() {
    let (provider, stub) = account_provider(submitted("job-1")).await;
    // A job stored at 480p: the JSON a store kept before Object Swap had its own resolutions.
    let mut object = serde_json::to_value(spec(Mode::ObjectSwap)).unwrap();
    object["resolution"] = json!("480p");
    let stored: JobSpec = serde_json::from_value(object).unwrap();
    assert_eq!(stored.resolution, Resolution::P720);
    assert_eq!(provider.estimate(&stored).await.unwrap(), credits("24", true));

    let specs = [
        spec_of(Mode::ObjectSwap, 12.0, Resolution::P480),
        stored,
        spec_of(Mode::MotionTransfer, 12.0, Resolution::P480),
    ];
    assert_eq!(specs[0].resolution, Resolution::P720);
    for spec in &specs {
        provider
            .submit(spec, &account_inputs(), Uuid::new_v4())
            .await
            .unwrap();
    }

    let params: Vec<Value> = calls(&stub)
        .await
        .iter()
        .map(|call| call["arguments"]["params"].clone())
        .filter(|params| !params.is_null())
        .collect();
    let resolutions: Vec<Value> = params.iter().map(|params| params["resolution"].clone()).collect();
    assert_eq!(resolutions, [json!("720p"), json!("720p"), json!("480p")]);
    for sent in &params {
        assert!(catalog::violations_of_params(sent).is_empty(), "{sent}");
    }
}

/// Up to 10 reference images in either mode, at least one.
#[test]
fn it_takes_up_to10_images() {
    for mode in [Mode::MotionTransfer, Mode::ObjectSwap] {
        let mut spec = spec(mode);
        spec.references = images(10, "/tmp/a.jpg");
        assert_none(&check(&spec));
        spec.references.extend(images(1, "/tmp/b.jpg"));
        assert_eq!(check(&spec), [ValidationIssue::TooManyReferences { maximum: 10 }]);
        spec.references = Vec::new();
        assert_eq!(check(&spec), [ValidationIssue::TooFewReferences { minimum: 1 }]);
    }
}

/// Billed by its source, like Genjutsu: `get_cost` quotes it once the source is uploaded, with the source in
/// `medias`. Before that it is the catalog's estimate, 2 credits a second at 720p, and nothing is asked.
#[tokio::test]
async fn it_is_quoted_once_the_source_is_uploaded() {
    for mode in [Mode::MotionTransfer, Mode::ObjectSwap] {
        let (provider, stub) =
            account_provider(json!({"cost": {"credits": 12, "credits_exact": 12.5}})).await;
        assert_eq!(provider.estimate(&spec(mode)).await.unwrap(), credits("24", true));
        // Object Swap asked for 480p is made, and estimated, at 720p.
        let low = if mode == Mode::ObjectSwap { "24" } else { "12" };
        assert_eq!(
            provider
                .estimate(&spec_of(mode, 12.0, Resolution::P480))
                .await
                .unwrap(),
            credits(low, true)
        );
        assert_none(&calls(&stub).await);

        let quoted = provider
            .server_estimate(&spec(mode), &account_inputs())
            .await
            .unwrap();
        assert_eq!(quoted, Some(credits("12.5", false)));
        let sent = calls(&stub).await[0]["arguments"]["params"].clone();
        assert_eq!(sent["get_cost"], true);
        assert_eq!(
            sent["model"].as_str(),
            account_model::model(&spec(mode)).as_deref()
        );
        assert_eq!(sent["medias"][0]["value"], "video-1");
        assert_none(&schema_violations(&stub).await);
    }
}

/// A stub REST host answering `answer` to every request.
async fn rest_host(answer: impl Fn(&wiremock::Request) -> ResponseTemplate + Send + Sync + 'static) -> Stub {
    Stub::serve(answer).await
}

/// The account runs both modes; an API key runs Motion Transfer alone, the one mode REST lists
/// (`higgsfield/genjutsu-light/motion-transfer/v1.0`), and refuses an Object Swap before anything is sent.
#[tokio::test]
async fn an_api_key_runs_motion_transfer_alone() {
    let (account, _stub) = account_provider(json!({})).await;
    let account_capabilities = account.capabilities().await.unwrap();
    assert!(account_model::runs(&lite()));
    assert!(account_capabilities.runs(&lite(), Mode::MotionTransfer));
    assert!(account_capabilities.runs(&lite(), Mode::ObjectSwap));

    let server = rest_host(|_| ResponseTemplate::new(404)).await;
    let capabilities = make_api_provider(&server, "key-id").capabilities().await.unwrap();
    assert!(lite().runs_with_api_key());
    assert_eq!(
        capabilities.unserved_modes,
        BTreeMap::from([(lite(), BTreeSet::from([Mode::ObjectSwap]))])
    );
    assert!(capabilities.runs(&lite(), Mode::MotionTransfer));
    assert!(!capabilities.runs(&lite(), Mode::ObjectSwap));
    assert!(matches!(
        ApiModel::new(&spec(Mode::ObjectSwap), &rest_inputs()),
        Err(PortError::Provider(_))
    ));
}

const QUEUED: &str = r#"{"status": "queued", "request_id": "req-1",
 "status_url": "https://api.higgsfield.ai/requests/req-1/status"}"#;

/// The body of its console.higgsfield.ai schema, Genjutsu's: `video_url`, `image_urls`, `prompt` and `resolution`,
/// sent to its own endpoint.
#[tokio::test]
async fn an_api_key_sends_genjutsus_body_to_lites_endpoint() {
    for resolution in [Resolution::P480, Resolution::P720, Resolution::P1080] {
        let server = rest_host(|request| {
            if request.method.as_str() == "POST"
                && request.url.path() == "/higgsfield/genjutsu-light/motion-transfer/v1.0"
            {
                ResponseTemplate::new(200).set_body_raw(QUEUED, "application/json")
            } else {
                ResponseTemplate::new(404)
            }
        })
        .await;
        make_api_provider(&server, "key-id")
            .submit(
                &spec_of(Mode::MotionTransfer, 12.0, resolution),
                &rest_inputs(),
                Uuid::new_v4(),
            )
            .await
            .unwrap();

        let requests = server.requests().await;
        assert_eq!(
            body_json(&requests[0]),
            json!({
                "video_url": "https://cdn.example.com/in.mp4", "image_urls": ["https://cdn.example.com/a.jpg"],
                "prompt": "make it rain", "resolution": resolution.raw_value(),
            })
        );
    }
}

/// The list price bills the output's seconds and the input video's, both the selection's: 12 s at 720p is
/// 24 × $0.10. The source is uploaded, so nothing is quoted before.
#[tokio::test]
async fn an_api_key_estimates_the_list_price() {
    for (resolution, price) in [
        (Resolution::P480, "1.2"),
        (Resolution::P720, "2.4"),
        (Resolution::P1080, "4.8"),
    ] {
        let server = rest_host(|_| ResponseTemplate::new(404)).await;
        let cost = make_api_provider(&server, "key-id")
            .estimate(&spec_of(Mode::MotionTransfer, 12.0, resolution))
            .await
            .unwrap();
        assert_eq!(cost, usd(price, false));
        assert_eq!(server.requests().await.len(), 0);
    }
}

/// REST's schema takes 1-8 images, the account up to 10: an API key reports 8 as Lite's limit.
#[tokio::test]
async fn an_api_key_takes_up_to8_images() {
    let server = rest_host(|_| ResponseTemplate::new(404)).await;
    let table = make_api_provider(&server, "key-id")
        .model_constraints()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        lite().limits(Mode::MotionTransfer, true, &table).reference_images,
        ClosedRange::new(1, 8)
    );
    assert_eq!(
        VideoModel::GENJUTSU
            .limits(Mode::MotionTransfer, true, &table)
            .reference_images,
        ClosedRange::new(1, 8)
    );
    let mut spec = spec(Mode::MotionTransfer);
    spec.references = images(9, "/tmp/a.jpg");
    assert_eq!(
        Validation::check(&spec, &table),
        [ValidationIssue::TooManyReferences { maximum: 8 }]
    );
    assert_none(&check(&spec));
}

/// Its `models_explore` answers limit its own jobs, not Genjutsu's.
#[test]
fn the_servers_limits_apply_to_lite_alone() {
    let table = account_model::constraints(&HashMap::from([(
        "hf_mult_motion_control_flash".to_owned(),
        ModelLimits {
            max_duration: Some(10.0),
            ..ModelLimits::default()
        },
    )]));
    assert_eq!(
        table.entries(),
        [ModelConstraintEntry {
            model: lite(),
            mode: Mode::MotionTransfer,
            constraints: ModelConstraints::new(None, Some(10.0), None, None),
        }]
    );
    let duration = |model: &VideoModel, mode| model.limits(mode, true, &table).duration;
    assert_eq!(
        duration(&lite(), Mode::MotionTransfer),
        ClosedRange::new(4.0, 10.0)
    );
    assert_eq!(duration(&lite(), Mode::ObjectSwap), ClosedRange::new(4.0, 15.0));
    assert_eq!(
        duration(&VideoModel::GENJUTSU, Mode::MotionTransfer),
        ClosedRange::new(4.0, 30.0)
    );
}

/// The live catalog's answer states no length, so the catalog's 15 s stands; a longer selection is refused.
#[tokio::test]
async fn a_selection_past15_seconds_is_refused() {
    let motion = catalog::model("hf_mult_motion_control_flash");
    let swap = catalog::model("hf_mult_replace_object_flash");
    let mut tools = full_set();
    tools.push(tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "cursor"],
    ));
    let stub = McpServer::new(tools)
        .call(move |_, arguments| match arguments["model_id"].as_str() {
            Some("hf_mult_motion_control_flash") => motion.clone(),
            Some("hf_mult_replace_object_flash") => swap.clone(),
            _ => json!({}),
        })
        .serve()
        .await;
    let provider = make_provider(&stub, false);
    let table: ModelConstraintTable = provider.model_constraints().await.unwrap().unwrap();
    assert_eq!(
        table.get(&lite(), Mode::MotionTransfer),
        Some(ModelConstraints::default())
    );
    assert_eq!(
        lite().limits(Mode::MotionTransfer, true, &table).duration,
        ClosedRange::new(4.0, 15.0)
    );
    assert_none(&Validation::check(
        &spec_of(Mode::MotionTransfer, 15.0, Resolution::P720),
        &table,
    ));
    assert_eq!(
        Validation::check(&spec_of(Mode::MotionTransfer, 20.0, Resolution::P720), &table),
        [ValidationIssue::DurationTooLong(20.0)]
    );
    assert_eq!(
        Validation::check(&spec_of(Mode::ObjectSwap, 20.0, Resolution::P720), &table),
        [ValidationIssue::DurationTooLong(20.0)]
    );
}

/// A finished Lite job in the account's history comes back as Lite, in its mode.
#[test]
fn history_names_the_lite_model() {
    assert_eq!(
        history_mapping::kind_of_model("hf_mult_motion_control_flash").map(|kind| kind.1),
        Some(lite())
    );
    assert_eq!(
        history_mapping::kind_of_model("hf_mult_replace_object_flash").map(|kind| kind.0),
        Some(Mode::ObjectSwap)
    );
}
