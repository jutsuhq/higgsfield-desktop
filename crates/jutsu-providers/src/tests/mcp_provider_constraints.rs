//! Port of `MCPProviderConstraintsTests.swift`: `AccountModelConstraintsTests` and `Network.ProviderConstraints`.

use std::collections::HashMap;

use higgsfield::mcp::account::{ModelLimits, VideoRequest};
use jutsu_core::{
    ClosedRange, GenerationProvider, JobSpec, Mode, ModelConstraintEntry, ModelConstraintTable,
    ModelConstraints, ModelConstraintsSource, PixelSize, Resolution, SourceClip, VideoModel,
};
use serde_json::{Value, json};

use super::support::account::calls;
use super::support::agent_b::{assert_same, credits, file_url, provider_tools};
use super::support::catalog;
use super::support::mcp::{McpServer, tool};
use super::support::provider::make_provider;
use super::support::stub::Stub;
use crate::{HiggsfieldMcpProvider, account_model};

fn seedance45() -> ModelConstraintTable {
    ModelConstraintTable::new(vec![ModelConstraintEntry {
        model: VideoModel::SEEDANCE25,
        mode: Mode::MotionTransfer,
        constraints: ModelConstraints::new(None, Some(45.0), None, None),
    }])
}

/// Seedance on a 40 s selection.
fn spec() -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        60.0,
        PixelSize {
            width: 1280,
            height: 720,
        },
        ClosedRange::new(0.0, 40.0),
    );
    JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Some(source),
        Resolution::P720,
    )
}

#[test]
fn each_generate_model_limits_its_job() {
    let table = account_model::constraints(&HashMap::from([
        (
            VideoRequest::MOTION_CONTROL.to_owned(),
            ModelLimits {
                max_images: Some(4),
                ..ModelLimits::default()
            },
        ),
        (
            VideoRequest::REPLACE_OBJECT.to_owned(),
            ModelLimits {
                min_images: Some(2),
                ..ModelLimits::default()
            },
        ),
        (
            VideoRequest::SEEDANCE_25.to_owned(),
            ModelLimits {
                min_duration: Some(5.0),
                max_duration: Some(45.0),
                ..ModelLimits::default()
            },
        ),
    ]));
    assert_eq!(
        table.entries(),
        [
            ModelConstraintEntry {
                model: VideoModel::GENJUTSU,
                mode: Mode::MotionTransfer,
                constraints: ModelConstraints::new(None, None, None, Some(4)),
            },
            ModelConstraintEntry {
                model: VideoModel::GENJUTSU,
                mode: Mode::ObjectSwap,
                constraints: ModelConstraints::new(None, None, Some(2), None),
            },
            ModelConstraintEntry {
                model: VideoModel::SEEDANCE25,
                mode: Mode::MotionTransfer,
                constraints: ModelConstraints::new(Some(5.0), Some(45.0), None, None),
            },
        ]
    );
}

#[test]
fn the_request_length_keeps_to_the_table() {
    assert_eq!(
        account_model::duration_seconds(&spec(), &ModelConstraintTable::built_in()),
        30
    );
    assert_eq!(account_model::duration_seconds(&spec(), &seedance45()), 40);
    let request = account_model::request(
        &spec(),
        VideoRequest::SEEDANCE_25,
        Vec::new(),
        &account_model::parameters(),
        &seedance45(),
    );
    assert_eq!(request.duration, Some(40.0));
}

/// `ProviderConstraints.explore`.
fn explore() -> Value {
    tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "cursor"],
    )
}

fn tools_with_explore() -> Vec<Value> {
    let mut tools = provider_tools();
    tools.push(explore());
    tools
}

/// `ProviderConstraints.provider`: the account provider with the app's cached `constraints`.
async fn provider(
    tools: Vec<Value>,
    constraints: ModelConstraintTable,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(tools).call(call).serve().await;
    let provider = make_provider(&stub, false).with_constraints(move || {
        let constraints = constraints.clone();
        async move { constraints }
    });
    (provider, stub)
}

/// The fallback estimate and the quote before upload both bill the length the table allows.
#[tokio::test]
async fn the_estimate_bills_the_length_the_table_allows() {
    let (built_in, _built_in_stub) = provider(provider_tools(), ModelConstraintTable::built_in(), |_, _| {
        json!({})
    })
    .await;
    let (cached, _cached_stub) = provider(provider_tools(), seedance45(), |_, _| json!({})).await;
    assert_eq!(built_in.estimate(&spec()).await.unwrap(), credits("210", true));
    assert_eq!(cached.estimate(&spec()).await.unwrap(), credits("280", true));

    let (quoted, stub) = provider(provider_tools(), seedance45(), |_, arguments| {
        json!({"cost": {"credits": arguments["params"]["duration"].as_f64().unwrap_or(0.0) * 7.0}})
    })
    .await;
    assert_eq!(quoted.estimate(&spec()).await.unwrap(), credits("280", false));
    let durations: Vec<Value> = calls(&stub)
        .await
        .into_iter()
        .map(|call| call["arguments"]["params"]["duration"].clone())
        .collect();
    assert_same(&Value::Array(durations), &json!([40]));
}

#[tokio::test]
async fn reads_the_constraints_from_models_explore() {
    let (provider, _stub) = provider(tools_with_explore(), ModelConstraintTable::built_in(), |_, arguments| {
        if arguments["model_id"] != "kling3_0_motion_control" {
            return json!({});
        }
        json!({"parameters": {"duration": {"minimum": 3, "maximum": 10}}, "medias": [{"role": "image", "max": 1}]})
    })
    .await;
    let table = provider.model_constraints().await.unwrap().unwrap();
    assert_eq!(
        table.get(&VideoModel::KLING_MOTION_CONTROL, Mode::MotionTransfer),
        Some(ModelConstraints::new(Some(3.0), Some(10.0), None, Some(1)))
    );
    assert_eq!(
        table.get(&VideoModel::SEEDANCE25, Mode::MotionTransfer),
        Some(ModelConstraints::default())
    );
}

/// The snapshot's answers: Seedance's Motion job gets 4–30 s, the rest keep the constants.
#[tokio::test]
async fn reads_the_snapshots_answers() {
    let seedance = catalog::value(catalog::SEEDANCE, &[]);
    let kling = catalog::value(catalog::KLING, &[]);
    let motion = catalog::value(catalog::MOTION_CONTROL, &[]);
    let (provider, _stub) = provider(
        tools_with_explore(),
        ModelConstraintTable::built_in(),
        move |_, arguments| match arguments["model_id"].as_str() {
            Some("seedance_2_5") => seedance.clone(),
            Some("kling3_0_motion_control") => kling.clone(),
            _ => motion.clone(),
        },
    )
    .await;
    let table = provider.model_constraints().await.unwrap().unwrap();
    assert_eq!(
        table.get(&VideoModel::SEEDANCE25, Mode::MotionTransfer),
        Some(ModelConstraints::new(Some(4.0), Some(30.0), None, None))
    );
    assert_eq!(
        table.get(&VideoModel::KLING_MOTION_CONTROL, Mode::MotionTransfer),
        Some(ModelConstraints::default())
    );
    assert_eq!(
        table.get(&VideoModel::GENJUTSU, Mode::MotionTransfer),
        Some(ModelConstraints::default())
    );
}

#[tokio::test]
async fn without_models_explore_there_are_no_constraints() {
    let (provider, _stub) = provider(provider_tools(), ModelConstraintTable::built_in(), |_, _| {
        json!({})
    })
    .await;
    assert_eq!(provider.model_constraints().await.unwrap(), None);
}
