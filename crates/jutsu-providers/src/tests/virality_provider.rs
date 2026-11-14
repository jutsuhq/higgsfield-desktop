//! Port of `ViralityProviderTests.swift`: virality scores (issues #45, #111), against the answers of a live run on
//! 2026-10-15 with synthetic IDs and URLs: `create` starts a free job, whose `job_status` carries the readings once
//! it completes.
//!
//! Not ported: `theAccountOffersScores` reads Swift's static `offersVirality` flag, which has no Rust counterpart
//! (the provider implements `ViralityScoring` instead).

use std::time::Duration;

use jutsu_core::{Cost, PortError, ProviderError, ViralityScoring};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use url::Url;

use super::support::account::{calls, calls_of, full_set, schema_violations};
use super::support::agent_d::provider as account_provider;
use super::support::catalog;
use super::support::mcp::Locked;
use super::support::stub::Stub;
use crate::HiggsfieldMcpProvider;
use crate::account_virality::{MAXIMUM_POLLS, TIMED_OUT, pause};

const VIDEO_ID: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";
const JOB_ID: &str = "7d2e9b41-0c3a-4f6e-8b12-5a9c0d1e2f34";
const DASHBOARD: &str = "https://cdn.example/virality/hf_20261015_job.html";
const DISCLAIMER: &str = "Predictive proxy metrics, not guaranteed performance or clinical measures.";

/// `poll_interval` is 0 here so the test doesn't wait; the live answer said 1.5.
fn created() -> Value {
    json!({"job_id": JOB_ID, "model": "virality_predictor", "poll_interval": 0})
}

fn generation(status: &str, analysis: &Value, error: Option<&str>) -> Value {
    json!({
        "generation": {
            "id": JOB_ID, "type": "image", "status": status, "model": "brain_activity",
            "params": {
                "medias": [{"role": "video", "data": {"id": VIDEO_ID}}], "analysis": analysis.clone(),
            },
            "results": if status == "completed" { json!({"rawUrl": DASHBOARD}) } else { Value::Null },
            "error": error,
        },
    })
}

fn analysis() -> Value {
    json!({
        "scores": {
            "sustain": 100, "hook_score": 29, "peak_score": 0.42, "peak_second": 12, "overall_score": 46,
            "viral_potential": 45, "brain_engagement": 39, "peak_frame_index": 12,
        },
        "summary": {"overall_score": 39, "sustain_score": 100},
        "score_details": {"disclaimer": DISCLAIMER, "hook_window_seconds": [0, 3]},
        "global_scores_by_frame": [0.2, 0.42, 0.3],
    })
}

/// The stub answers `create` with `create`, then each `job_status` with the next of `statuses`, the last one
/// repeated.
async fn provider(statuses: Vec<Value>, create: Value) -> (HiggsfieldMcpProvider, Stub) {
    let answered = Locked::new(0_usize);
    let mut tools = full_set();
    tools.push(catalog::tool("virality_predictor"));
    account_provider(tools, move |name, _| match name {
        "virality_predictor" => create.clone(),
        "job_status" => {
            let index = answered.update(|count| {
                *count += 1;
                *count - 1
            });
            statuses
                .get(index.min(statuses.len().saturating_sub(1)))
                .cloned()
                .unwrap_or_else(|| json!({}))
        }
        _ => json!({}),
    })
    .await
}

fn free() -> Cost {
    Cost::credits(Decimal::ZERO, false)
}

#[tokio::test]
async fn scoring_is_free_and_asks_nothing() {
    let (provider, stub) = provider(vec![generation("queued", &Value::Null, None)], created()).await;
    assert_eq!(provider.virality_estimate(VIDEO_ID).await, Ok(Some(free())));
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn the_job_is_polled_until_it_completes_and_its_scores_kept() {
    let (provider, stub) = provider(
        vec![
            generation("queued", &Value::Null, None),
            generation("in_progress", &Value::Null, None),
            generation("completed", &analysis(), None),
        ],
        created(),
    )
    .await;

    let outcome = provider.score_virality(VIDEO_ID).await.unwrap();

    let score = outcome.score.unwrap();
    assert_eq!(score.score, Some(46));
    assert_eq!(score.viral_potential, Some(45));
    assert_eq!(score.hook, Some(29));
    assert_eq!(score.engagement, Some(39));
    assert_eq!(score.sustain, Some(100));
    assert_eq!(score.peak_second, Some(12.0));
    assert_eq!(score.per_frame, Some(vec![0.2, 0.42, 0.3]));
    assert_eq!(score.disclaimer.as_deref(), Some(DISCLAIMER));
    assert_eq!(score.report_url, Some(Url::parse(DASHBOARD).unwrap()));
    assert_eq!(outcome.charged, Some(free()));
    let predictions: Vec<Value> = calls_of(&stub, "virality_predictor")
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(
        predictions,
        vec![json!({
            "action": "create",
            "params": {"model": "virality_predictor", "medias": [{"role": "video", "id": VIDEO_ID}]},
        })]
    );
    let polls: Vec<Value> = calls_of(&stub, "job_status")
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(polls, vec![json!({"jobId": JOB_ID}); 3]);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_completed_job_without_scores_keeps_none() {
    let (provider, _stub) = provider(vec![generation("completed", &Value::Null, None)], created()).await;
    let outcome = provider.score_virality(VIDEO_ID).await.unwrap();
    assert_eq!(outcome.score, None);
    assert_eq!(outcome.charged, Some(free()));
}

#[tokio::test]
async fn a_failed_job_or_a_refusal_is_a_validation_error() {
    let validation = |message: &str| {
        Err(PortError::Provider(ProviderError::Validation {
            field: None,
            message: message.to_owned(),
        }))
    };
    let (failing, _failing_stub) = provider(
        vec![generation("failed", &Value::Null, Some("Video is too long"))],
        created(),
    )
    .await;
    assert_eq!(
        failing.score_virality(VIDEO_ID).await,
        validation("Video is too long")
    );

    let (refusing, _refusing_stub) = provider(Vec::new(), json!({"error": "Too long"})).await;
    assert_eq!(refusing.score_virality(VIDEO_ID).await, validation("Too long"));
}

#[tokio::test]
async fn a_job_that_never_ends_gives_up() {
    let (provider, stub) = provider(vec![generation("in_progress", &Value::Null, None)], created()).await;
    assert_eq!(
        provider.score_virality(VIDEO_ID).await,
        Err(PortError::Provider(ProviderError::Validation {
            field: None,
            message: TIMED_OUT.to_owned()
        }))
    );
    assert_eq!(calls_of(&stub, "job_status").await.len(), MAXIMUM_POLLS);
}

#[test]
fn the_pause_follows_the_answer_within_bounds() {
    assert_eq!(pause(None), Duration::from_secs_f64(1.5));
    assert_eq!(pause(Some(3.0)), Duration::from_secs(3));
    assert_eq!(pause(Some(120.0)), Duration::from_secs(10));
}
