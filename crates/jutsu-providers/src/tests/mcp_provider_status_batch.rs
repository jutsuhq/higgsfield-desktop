//! Port of `MCPProviderStatusBatchTests.swift`: `jobs_wait` through the provider, one call for up to 12 jobs, and
//! `None` (each job polled alone) whenever the tool is missing, fails or answers in a shape that can't be read.

use std::collections::BTreeMap;

use jutsu_core::{BatchStatus, GenerationProvider, RemoteJob, RemoteStatus};
use serde_json::{Value, json};
use url::Url;
use wiremock::Request;

use super::support::account::{calls, calls_of};
use super::support::agent_b::{
    assert_same, called_tool, provider, provider_tools, quiet_provider, reply_with,
};
use super::support::mcp::{McpServer, Reply, tool};
use super::support::provider::make_provider;
use super::support::stub::Stub;
use crate::HiggsfieldMcpProvider;

fn jobs_wait() -> Value {
    tool(
        "jobs_wait",
        "Wait for up to 12 jobs",
        &["jobs", "timeout_seconds"],
    )
}

fn tools_with_jobs_wait() -> Vec<Value> {
    let mut tools = provider_tools();
    tools.push(jobs_wait());
    tools
}

fn jobs() -> Vec<RemoteJob> {
    vec![RemoteJob::new("job-1"), RemoteJob::new("job-2")]
}

async fn jobs_wait_calls(stub: &Stub) -> usize {
    calls_of(stub, "jobs_wait").await.len()
}

/// The MCP stub, except that every `jobs_wait` call gets `response(request)`.
async fn provider_answering(
    response: impl Fn(&Request) -> Reply + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    let mcp = McpServer::new(tools_with_jobs_wait());
    let stub = Stub::replying(move |request| {
        if called_tool(request).as_deref() == Some("jobs_wait") {
            response(request)
        } else {
            mcp.respond(request)
        }
    })
    .await;
    (make_provider(&stub, false), stub)
}

#[tokio::test]
async fn one_call_answers_every_job() {
    let (provider, stub) = provider(tools_with_jobs_wait(), |name, _| {
        if name == "jobs_wait" {
            json!({
                "jobs": [
                    {"index": 0, "job_id": "job-1", "status": "completed", "result_url": "https://cdn.example/1.mp4"},
                    {"index": 1, "job_id": "job-2", "status": "in_progress"},
                ],
                "poll_after_seconds": 4, "all_terminal": false,
            })
        } else {
            json!({})
        }
    })
    .await;

    let batch = provider.statuses(&jobs()).await.unwrap().unwrap();

    let output = Url::parse("https://cdn.example/1.mp4").unwrap();
    assert_eq!(
        batch,
        BatchStatus {
            statuses: BTreeMap::from([
                ("job-1".to_owned(), RemoteStatus::Completed { video_url: output }),
                ("job-2".to_owned(), RemoteStatus::InProgress),
            ]),
            poll_after: Some(time::Duration::seconds(4)),
        }
    );
    assert_eq!(provider.status_batch_limit(), 12);
    let calls = calls(&stub).await;
    let names: Vec<&Value> = calls.iter().map(|call| &call["name"]).collect();
    assert_eq!(names, [&json!("jobs_wait")]);
    assert_same(
        &calls[0]["arguments"],
        &json!({"jobs": [{"job_id": "job-1", "index": 0}, {"job_id": "job-2", "index": 1}]}),
    );
}

#[tokio::test]
async fn without_jobs_wait_each_job_is_polled_alone() {
    let (provider, stub) = quiet_provider(provider_tools()).await;
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
    assert_eq!(jobs_wait_calls(&stub).await, 0);
}

#[tokio::test]
async fn a_failed_call_falls_back_this_time_only() {
    let (provider, stub) = provider_answering(|_| Reply::status(500)).await;
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
    assert_eq!(jobs_wait_calls(&stub).await, 2);
}

#[tokio::test]
async fn a_tool_error_falls_back() {
    let (provider, _stub) = provider_answering(|request| {
        reply_with(
            request,
            &json!({"error": {"code": -32000, "message": "jobs_wait is busy"}}),
        )
    })
    .await;
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
}

#[tokio::test]
async fn an_unreadable_answer_stops_using_the_tool() {
    let (provider, stub) = provider(tools_with_jobs_wait(), |name, _| {
        if name == "jobs_wait" {
            json!({"state": {"job-1": "done"}})
        } else {
            json!({})
        }
    })
    .await;
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
    assert_eq!(provider.statuses(&jobs()).await.unwrap(), None);
    assert_eq!(jobs_wait_calls(&stub).await, 1);
}
