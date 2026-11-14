//! Port of `MCPProviderVariantsTests.swift`: variants are `generate_video`'s `count` (catalog snapshot of
//! 2026-10-15).

use jutsu_core::{GenerationProvider, JobSpec, Mode, PortError};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::account::{calls, schema_violations};
use super::support::agent_b::{
    credits, generate_params, inputs, provider, provider_tools, quiet_provider, unlimited_page,
    unlimited_tools,
};
use super::support::mcp::McpServer;
use super::support::provider::{make_api_provider, make_provider};

fn spec(mode: Mode, variants: i64, uses_unlimited: bool) -> JobSpec {
    let mut spec = super::support::agent_b::spec(mode);
    spec.variants = variants;
    spec.uses_unlimited = uses_unlimited;
    spec
}

#[tokio::test]
async fn capabilities_offer_four_variants_where_the_account_generates() {
    let (provider, _stub) = quiet_provider(provider_tools()).await;
    assert_eq!(provider.capabilities().await.unwrap().max_variants, 4);
    let (without_generate, _other) = quiet_provider(Vec::new()).await;
    assert_eq!(without_generate.capabilities().await.unwrap().max_variants, 1);
}

#[tokio::test]
async fn every_job_of_the_answer_comes_back() {
    let (provider, stub) = provider(provider_tools(), |_, _| {
        json!({"results": [{"id": "job-1"}, {"id": "job-2"}, {"id": "job-3"}], "request_id": "trace-1"})
    })
    .await;
    let job = provider
        .submit(&spec(Mode::MotionTransfer, 3, false), &inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(job.request_id, "job-1");
    let variants: Vec<&str> = job
        .variants
        .iter()
        .map(|variant| variant.request_id.as_str())
        .collect();
    assert_eq!(variants, ["job-2", "job-3"]);
    assert!(
        job.variants
            .iter()
            .all(|variant| variant.correlation_id.as_deref() == Some("trace-1"))
    );
    assert_eq!(generate_params(&stub).await[0]["count"], 3);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

/// The server caps an unlimited request to one generation: no count is sent with `use_unlim`.
#[tokio::test]
async fn an_unlimited_job_sends_no_count() {
    let stub = McpServer::new(unlimited_tools())
        .call(|name, _| match name {
            "models_explore" => unlimited_page(true),
            "generate_video" => json!({"results": [{"id": "job-1"}]}),
            _ => json!({}),
        })
        .serve()
        .await;
    let provider = make_provider(&stub, true);
    provider.capabilities().await.unwrap();

    provider
        .submit(&spec(Mode::MotionTransfer, 3, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();

    let params = generate_params(&stub).await.remove(0);
    assert_eq!(params["use_unlim"], true);
    assert!(params.get("count").is_none());
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn the_estimate_is_one_variants_times_the_count() {
    let (provider, _stub) = quiet_provider(provider_tools()).await;
    let one = provider
        .estimate(&spec(Mode::MotionTransfer, 1, false))
        .await
        .unwrap();
    let three = provider
        .estimate(&spec(Mode::MotionTransfer, 3, false))
        .await
        .unwrap();
    assert_eq!(three.amount(), one.amount() * Decimal::from(3));
}

/// Whether `get_cost` prices `count` isn't documented: one variant is quoted and priced that many times.
#[tokio::test]
async fn the_quote_is_for_one_variant_times_the_count() {
    let (provider, stub) = provider(
        provider_tools(),
        |_, _| json!({"cost": {"credits": 28, "credits_exact": 27.5}}),
    )
    .await;
    let cost = provider
        .server_estimate(&spec(Mode::MotionTransfer, 3, false), &inputs())
        .await
        .unwrap();

    assert_eq!(cost, Some(credits("82.5", false)));
    let params = generate_params(&stub).await.remove(0);
    assert_eq!(params["get_cost"], true);
    assert!(params.get("count").is_none());
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

/// A Genjutsu Style runs through `execute_preset`, which has no count: refused before anything is sent.
#[tokio::test]
async fn a_style_of_several_variants_is_refused_before_sending() {
    let (provider, stub) = quiet_provider(provider_tools()).await;
    let mut spec = spec(Mode::Restyle, 2, false);
    spec.preset_id = Some(Uuid::new_v4());
    let error = provider
        .submit(&spec, &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(error, PortError::Provider(_)), "{error:?}");
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn the_api_key_refuses_variants_before_sending() {
    let (_account, stub) = quiet_provider(Vec::new()).await;
    let provider = make_api_provider(&stub, "key-id");
    let error = provider
        .estimate(&spec(Mode::MotionTransfer, 2, false))
        .await
        .unwrap_err();
    assert!(matches!(error, PortError::Provider(_)), "{error:?}");
    let error = provider
        .submit(&spec(Mode::MotionTransfer, 2, false), &inputs(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(error, PortError::Provider(_)), "{error:?}");
    assert!(stub.requests().await.is_empty());
}
