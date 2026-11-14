//! The network half of `ViralityTests`: the answers of a live Virality Predictor run on 2026-10-15, with synthetic
//! IDs and URLs.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{HiggsfieldAccount, ViralityJob, ViralityPrediction, ViralityStatus};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, calls, full_set, schema_violations};
use crate::support::catalog;
use crate::support::mcp::McpServer;
use crate::support::stub::Stub;

const VIDEO_ID: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";
const JOB_ID: &str = "7d2e9b41-0c3a-4f6e-8b12-5a9c0d1e2f34";
const DASHBOARD: &str = "https://cdn.example/virality/hf_20261015_job.html";
const DISCLAIMER: &str = "Predictive proxy metrics, not guaranteed performance or clinical measures.";

fn generation(status: &str, analysis: &Value, results: &Value) -> Value {
    json!({"generation": {
        "id": JOB_ID, "type": "image", "status": status, "model": "brain_activity",
        "params": {
            "prompt": "",
            "medias": [{"role": "video", "data": {"id": VIDEO_ID, "type": "video_input"}}],
            "video_meta": {"duration": 12}, "analysis": analysis,
        },
        "results": results,
    }})
}

fn analysis() -> Value {
    json!({
        "scores": {
            "sustain": 100, "hook_score": 29, "peak_score": 0.42, "peak_second": 12, "overall_score": 46,
            "viral_potential": 45, "brain_engagement": 39, "peak_frame_index": 12,
        },
        "summary": {"overall_score": 39, "sustain_score": 100},
        "regions": [{
            "id": "visual", "title": "Visual", "subtitle": "Visual cortex", "mean_score": 0.31,
            "peak_score": 0.5, "peak_second": 4, "lower_better": false, "values_by_frame": [0.1, 0.5, 0.3],
        }],
        "timeline": {"frame_count": 3, "time_seconds": [0, 1, 2], "duration_seconds": 12, "captions_by_frame": ["", "", ""]},
        "score_details": {
            "disclaimer": DISCLAIMER,
            "normalization": "scores are normalized prediction proxies in range 0..100", "hook_window_seconds": [0, 3],
        },
        "global_scores_by_frame": [0.2, 0.42, 0.3],
    })
}

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.push(catalog::tool("virality_predictor"));
    tools
}

async fn virality(answer: Value) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(tools()).call(move |_, _| answer.clone())).await
}

#[tokio::test]
async fn create_names_the_video_as_the_schema_says_and_reads_the_job() {
    let (account, stub) =
        virality(json!({"job_id": JOB_ID, "model": "virality_predictor", "poll_interval": 1.5})).await;

    let job = account.predict_virality(VIDEO_ID).await.unwrap();

    assert_eq!(
        job,
        ViralityJob {
            id: JOB_ID.into(),
            poll_interval: Some(1.5)
        }
    );
    assert_eq!(
        calls(&stub).await,
        [json!({
            "name": "virality_predictor",
            "arguments": {
                "action": "create",
                "params": {"model": "virality_predictor", "medias": [{"role": "video", "id": VIDEO_ID}]},
            },
        })]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_running_job_has_no_analysis_yet() {
    let (account, stub) = virality(generation("in_progress", &Value::Null, &Value::Null)).await;
    assert_eq!(account.virality_status(JOB_ID).await, Ok(ViralityStatus::Running));
    assert_eq!(calls(&stub).await[0]["arguments"], json!({"jobId": JOB_ID}));
}

#[tokio::test]
async fn a_completed_job_reads_the_scores_not_the_summary() {
    let (account, _stub) = virality(generation(
        "completed",
        &analysis(),
        &json!({"rawUrl": DASHBOARD}),
    ))
    .await;

    let status = account.virality_status(JOB_ID).await.unwrap();

    let expected = ViralityPrediction {
        overall: Some(46.0),
        viral_potential: Some(45.0),
        hook: Some(29.0),
        engagement: Some(39.0),
        sustain: Some(100.0),
        peak_second: Some(12.0),
        per_frame: vec![0.2, 0.42, 0.3],
        disclaimer: Some(DISCLAIMER.into()),
        dashboard_url: Some(Url::parse(DASHBOARD).unwrap()),
    };
    assert_eq!(status, ViralityStatus::Completed(Some(expected)));
}

#[tokio::test]
async fn an_error_without_a_job_is_a_refusal() {
    let (account, _stub) = virality(json!({"error": "Video is longer than 15 seconds"})).await;
    assert_eq!(
        account.predict_virality(VIDEO_ID).await,
        Err(HiggsfieldError::validation("Video is longer than 15 seconds"))
    );
}

#[tokio::test]
async fn a_missing_tool_is_reported_without_a_call() {
    let (account, stub) = account(McpServer::new(full_set())).await;
    assert_eq!(
        account.predict_virality(VIDEO_ID).await,
        Err(HiggsfieldError::ToolUnavailable("virality_predictor".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}
