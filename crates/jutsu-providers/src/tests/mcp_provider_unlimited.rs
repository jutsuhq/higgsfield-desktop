//! Port of `MCPProviderUnlimitedTests.swift`: `models_explore` `list` answers in the shape of the catalog snapshot
//! of 2026-10-15.

use std::collections::{BTreeMap, BTreeSet};

use jutsu_core::{GenerationProvider, JobSpec, Mode, VideoModel};
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::account::calls_of;
use super::support::agent_b::{
    credits, generate_params, inputs, provider_tools, unlimited_page, unlimited_tools,
};
use super::support::mcp::McpServer;
use super::support::provider::make_provider;
use super::support::stub::Stub;
use crate::HiggsfieldMcpProvider;

/// `ProviderUnlimited.provider`: `models_explore` answers `unlimited_page(available)`, `generate_video` answers
/// `generate`.
async fn unlimited_provider(
    opted_in: bool,
    available: bool,
    tools: Vec<Value>,
    generate: Value,
) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(tools)
        .call(move |name, _| match name {
            "models_explore" => unlimited_page(available),
            "generate_video" => generate.clone(),
            "balance" => json!({"credits": 100}),
            _ => json!({}),
        })
        .serve()
        .await;
    (make_provider(&stub, opted_in), stub)
}

fn job() -> Value {
    json!({"results": [{"id": "job-1"}]})
}

/// Opted in, with an allowance, on the tools with `models_explore`.
async fn default_provider() -> (HiggsfieldMcpProvider, Stub) {
    unlimited_provider(true, true, unlimited_tools(), job()).await
}

fn spec(mode: Mode, uses_unlimited: bool) -> JobSpec {
    let mut spec = super::support::agent_b::spec(mode);
    spec.uses_unlimited = uses_unlimited;
    spec
}

async fn use_unlim(stub: &Stub) -> Vec<Value> {
    generate_params(stub)
        .await
        .into_iter()
        .map(|params| params["use_unlim"].clone())
        .collect()
}

#[tokio::test]
async fn capabilities_list_what_the_allowance_covers() {
    let (provider, _stub) = default_provider().await;
    assert_eq!(
        provider.capabilities().await.unwrap().unlimited,
        BTreeMap::from([(VideoModel::GENJUTSU, BTreeSet::from([Mode::MotionTransfer]))])
    );
}

#[tokio::test]
async fn capabilities_without_models_explore_or_an_allowance_list_nothing() {
    let (without_tool, _stub) = unlimited_provider(true, true, provider_tools(), job()).await;
    assert!(without_tool.capabilities().await.unwrap().unlimited.is_empty());
    let (without_allowance, _other) = unlimited_provider(true, false, unlimited_tools(), job()).await;
    assert!(
        without_allowance
            .capabilities()
            .await
            .unwrap()
            .unlimited
            .is_empty()
    );
}

#[tokio::test]
async fn decision() {
    let cases = [
        (true, true, Mode::MotionTransfer, true),   // all yes
        (false, true, Mode::MotionTransfer, false), // setting off
        (true, true, Mode::ObjectSwap, false),      // model doesn't support it
        (true, false, Mode::MotionTransfer, false), // no allowance now
        (true, true, Mode::Restyle, false),         // a Genjutsu Style has no use_unlim
    ];
    for (opted_in, available, mode, pays) in cases {
        let (provider, _stub) = unlimited_provider(opted_in, available, unlimited_tools(), job()).await;
        provider.capabilities().await.unwrap();
        assert_eq!(
            provider.pays_unlimited(&spec(mode, false)).await,
            pays,
            "{opted_in} {available} {mode:?}"
        );
    }
}

#[tokio::test]
async fn nothing_is_paid_unlimited_before_the_allowance_was_read() {
    let (provider, stub) = default_provider().await;
    assert!(!provider.pays_unlimited(&spec(Mode::MotionTransfer, false)).await);
    assert_eq!(super::support::account::calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn use_unlim_is_sent_only_for_a_job_queued_for_it_that_still_qualifies() {
    let (provider, stub) = default_provider().await;
    provider.capabilities().await.unwrap();

    provider
        .submit(&spec(Mode::MotionTransfer, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    provider
        .submit(&spec(Mode::MotionTransfer, false), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    provider
        .submit(&spec(Mode::ObjectSwap, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(use_unlim(&stub).await, [json!(true), json!(false), json!(false)]);
}

#[tokio::test]
async fn turning_the_setting_off_after_queueing_sends_credits() {
    let (provider, stub) = unlimited_provider(false, true, unlimited_tools(), job()).await;
    provider.capabilities().await.unwrap();

    provider
        .submit(&spec(Mode::MotionTransfer, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();

    assert_eq!(use_unlim(&stub).await, [json!(false)]);
}

#[tokio::test]
async fn an_answer_without_a_price_is_paid_from_the_allowance() {
    let (provider, _stub) = default_provider().await;
    provider.capabilities().await.unwrap();
    let job = provider
        .submit(&spec(Mode::MotionTransfer, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    assert!(job.paid_with_unlimited);
    assert_eq!(job.charged, None);
}

#[tokio::test]
async fn an_answer_with_credits_records_the_real_price() {
    let generate = json!({"results": [{"id": "job-1"}], "cost": {"credits_exact": 39.2}});
    let (provider, _stub) = unlimited_provider(true, true, unlimited_tools(), generate).await;
    provider.capabilities().await.unwrap();
    let job = provider
        .submit(&spec(Mode::MotionTransfer, true), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    assert!(!job.paid_with_unlimited);
    assert_eq!(job.charged, Some(credits("39.2", false)));
}

#[tokio::test]
async fn a_credit_job_ignores_the_price_in_the_answer() {
    let generate = json!({"results": [{"id": "job-1"}], "cost": {"credits": 40}});
    let (provider, _stub) = unlimited_provider(true, true, unlimited_tools(), generate).await;
    provider.capabilities().await.unwrap();
    let job = provider
        .submit(&spec(Mode::MotionTransfer, false), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    assert!(!job.paid_with_unlimited);
    assert_eq!(job.charged, None);
}

#[tokio::test]
async fn the_balance_after_a_job_reads_the_allowance_again_while_there_is_one() {
    let (provider, stub) = default_provider().await;
    provider.capabilities().await.unwrap();
    provider.balance().await.unwrap();
    assert_eq!(calls_of(&stub, "models_explore").await.len(), 2);

    let (without, without_stub) = unlimited_provider(true, false, unlimited_tools(), job()).await;
    without.capabilities().await.unwrap();
    without.balance().await.unwrap();
    assert_eq!(calls_of(&without_stub, "models_explore").await.len(), 1);
}
