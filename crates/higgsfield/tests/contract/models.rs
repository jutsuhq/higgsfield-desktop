//! `MediaImportTests` and `ModelLimitsTests`: importing a link, and the limits `models_explore get` states.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{HiggsfieldAccount, ModelLimits, VideoRequest};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, calls, calls_of, full_set};
use crate::support::catalog;
use crate::support::mcp::{McpServer, tool};
use crate::support::stub::Stub;

async fn importing(
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    let mut tools = full_set();
    tools.push(catalog::tool("media_import_url"));
    account(McpServer::new(tools).call(call)).await
}

fn link() -> Url {
    Url::parse("https://videos.example.com/clip.mp4").unwrap()
}

#[tokio::test]
async fn an_answer_without_a_media_id_is_a_refusal() {
    let (account, _stub) = importing(|_, _| json!({"error": "File is over 50 MB"})).await;
    assert_eq!(
        account.import_media(&link(), "video").await,
        Err(HiggsfieldError::validation("File is over 50 MB"))
    );
}

#[tokio::test]
async fn a_missing_import_tool_is_reported() {
    let (account, stub) = account(McpServer::new(full_set())).await;
    assert_eq!(
        account.import_media(&link(), "image").await,
        Err(HiggsfieldError::ToolUnavailable("media_import_url".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

// `models_explore get` answers in shapes other than the live one, still read tolerantly.

fn limits(
    min_duration: Option<f64>,
    max_duration: Option<f64>,
    min_images: Option<i64>,
    max_images: Option<i64>,
) -> ModelLimits {
    ModelLimits {
        min_duration,
        max_duration,
        min_images,
        max_images,
    }
}

/// Seedance 2.5 states its duration, 4–30 s; no answer counts the reference images, and Kling 3.0 Motion Control
/// and Genjutsu state no duration, so those bounds keep the app's constants.
#[test]
fn the_snapshots_answers_are_read() {
    let seedance = ModelLimits::from_answer(&catalog::value(catalog::SEEDANCE, &[]));
    assert_eq!(seedance, limits(Some(4.0), Some(30.0), None, None));
    assert_eq!(
        ModelLimits::from_answer(&catalog::value(catalog::KLING, &[])),
        ModelLimits::default()
    );
    assert_eq!(
        ModelLimits::from_answer(&catalog::value(catalog::MOTION_CONTROL, &[])),
        ModelLimits::default()
    );
}

#[test]
fn an_object_of_parameters_is_read_too() {
    let seedance = json!({
        "id": "seedance_2_5",
        "parameters": {
            "duration": {"type": "integer", "minimum": 4, "maximum": 30, "default": 5},
            "resolution": {"enum": ["480p", "720p", "1080p"]},
        },
        "medias": [
            {"role": "video_references", "min": 0, "max": 3},
            {"role": "image_references", "min": 0, "max": 9},
        ],
        "supports_unlim": false,
    });
    assert_eq!(
        ModelLimits::from_answer(&seedance),
        limits(Some(4.0), Some(30.0), Some(0), Some(9))
    );
}

#[test]
fn other_limit_shapes_are_read_too() {
    let nested = json!({"model": {
        "params": {"properties": {"duration": {"min": 3, "max": 12}}},
        "media": [{"roles": ["image_references", "image"], "min_count": 1, "max_count": 1}],
    }});
    assert_eq!(
        ModelLimits::from_answer(&nested),
        limits(Some(3.0), Some(12.0), Some(1), Some(1))
    );

    let listed = json!({"data": {
        "parameters": [{"name": "prompt"}, {"name": "duration", "enum": [5, 6, 7, 8]}],
        "medias": [{"type": "image", "maxItems": 4}],
    }});
    assert_eq!(
        ModelLimits::from_answer(&listed),
        limits(Some(5.0), Some(8.0), None, Some(4))
    );
}

#[test]
fn an_enum_with_gaps_is_no_range() {
    let answer = json!({"parameters": {"duration": {"enum": [5, 10]}}});
    assert_eq!(ModelLimits::from_answer(&answer), ModelLimits::default());
}

#[test]
fn frames_are_not_reference_images() {
    let answer = json!({"medias": [{"role": "start_image", "max": 1}, {"role": "end_image", "max": 1}]});
    assert_eq!(ModelLimits::from_answer(&answer), ModelLimits::default());
}

#[test]
fn junk_limits_read_as_nothing() {
    for answer in [
        Value::Null,
        json!("seedance_2_5"),
        json!(42),
        json!([]),
        json!({}),
        json!({"parameters": "duration"}),
        json!({"parameters": {"duration": "4-30"}}),
        json!({"parameters": {"duration": {"minimum": "4", "maximum": null}}}),
        json!({"parameters": {"duration": {"enum": ["5", "10"]}}}),
        json!({"parameters": {"duration": {"enum": [4.5, 5.5]}}}),
        json!({"parameters": {"duration": {"enum": [0, 1e300]}}}),
        json!({"medias": "image_references"}),
        json!({"medias": [{"role": "image_references", "max": -1, "min": 2.5}]}),
        json!({"medias": [{"role": 7, "max": 3}]}),
        json!({"model": "seedance_2_5"}),
    ] {
        assert_eq!(
            ModelLimits::from_answer(&answer),
            ModelLimits::default(),
            "{answer}"
        );
    }
}

fn explore() -> Value {
    tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "after", "limit"],
    )
}

fn video_models() -> Vec<String> {
    [
        VideoRequest::MOTION_CONTROL,
        VideoRequest::REPLACE_OBJECT,
        VideoRequest::SEEDANCE_25,
        VideoRequest::KLING_MOTION_CONTROL,
    ]
    .map(str::to_owned)
    .to_vec()
}

#[tokio::test]
async fn without_models_explore_there_is_no_answer() {
    let (account, stub) = account(McpServer::new(full_set())).await;
    assert_eq!(account.model_limits(&video_models()).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn gets_each_model() {
    let mut tools = full_set();
    tools.push(explore());
    let server = McpServer::new(tools).call(|_, arguments| match arguments["model_id"].as_str() {
        Some("seedance_2_5") => catalog::value(catalog::SEEDANCE, &[]),
        Some("kling3_0_motion_control") => catalog::value(catalog::KLING, &[]),
        _ => json!({"id": arguments["model_id"]}),
    });
    let (account, stub) = account(server).await;

    let limits = account.model_limits(&video_models()).await.unwrap().unwrap();

    assert_eq!(
        limits[VideoRequest::SEEDANCE_25],
        self::limits(Some(4.0), Some(30.0), None, None)
    );
    assert_eq!(limits[VideoRequest::KLING_MOTION_CONTROL], ModelLimits::default());
    let mut keys: Vec<_> = limits.keys().cloned().collect();
    keys.sort();
    let mut expected = video_models();
    expected.sort();
    assert_eq!(keys, expected);
    let calls: Vec<_> = calls_of(&stub, "models_explore")
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(calls.len(), video_models().len());
    let asked: Vec<_> = calls
        .iter()
        .filter_map(|arguments| arguments["model_id"].as_str())
        .collect();
    assert_eq!(asked, video_models());
    assert!(calls.iter().all(|arguments| arguments["action"] == "get"));
}
