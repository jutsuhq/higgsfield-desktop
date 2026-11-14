//! A `HiggsfieldAccount` on a stub MCP server, and readers of the calls it sent.

use std::sync::Arc;

use higgsfield::mcp::McpClient;
use higgsfield::mcp::account::HiggsfieldAccount;
use serde_json::Value;
use time::OffsetDateTime;
use wiremock::Request;

use super::catalog;
use super::mcp::{Locked, McpServer, Reply, fixture, method, token_provider, tool};
use super::stub::{Stub, body_json};

/// The tool contract as the live server gave it on 2026-10-14, trimmed to the keys the client reads.
pub fn full_set() -> Vec<Value> {
    vec![
        tool("generate_image", "Text to image", &["params"]),
        tool(
            "generate_video",
            "Generate one direct video request",
            &["params", "context"],
        ),
        tool(
            "media_upload",
            "Upload media for use in generation",
            &["filename", "content_type"],
        ),
        tool("media_confirm", "Confirm file uploads", &["media_id", "type"]),
        tool(
            "job_status",
            "Check the status and results of an async job",
            &["jobId", "sync"],
        ),
        tool(
            "balance",
            "Get the user's available credits and current plan",
            &[],
        ),
    ]
}

/// An account on `stub`, reading the time from `now` when given.
pub fn account_on(stub: &Stub, now: Option<Arc<Locked<OffsetDateTime>>>) -> HiggsfieldAccount {
    let client =
        McpClient::new(fixture::mcp_url(), token_provider("access-1")).with_http_client(stub.client());
    let account = HiggsfieldAccount::new(Arc::new(client)).with_http_client(stub.client());
    match now {
        Some(now) => account.with_clock(Arc::new(move || now.value())),
        None => account,
    }
}

/// An account on `server`.
pub async fn account(server: McpServer) -> (HiggsfieldAccount, Stub) {
    let stub = server.serve().await;
    (account_on(&stub, None), stub)
}

/// An account on the full tool set whose `tools/call` answers come from `call`.
pub async fn answering(
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(full_set()).call(call)).await
}

/// An account whose stub answers every request with `handler`, the MCP ones usually through an `McpServer`.
pub async fn account_replying(
    handler: impl Fn(&Request) -> Reply + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    let stub = Stub::replying(handler).await;
    (account_on(&stub, None), stub)
}

/// The `params` (`name` and `arguments`) of every `tools/call` the stub got.
pub async fn calls(stub: &Stub) -> Vec<Value> {
    stub.requests()
        .await
        .iter()
        .filter(|request| method(request).as_deref() == Some("tools/call"))
        .map(|request| body_json(request)["params"].clone())
        .collect()
}

/// The calls of tool `name`.
pub async fn calls_of(stub: &Stub, name: &str) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .filter(|call| call["name"] == name)
        .collect()
}

/// Where the calls the stub got break their tool's `inputSchema` in the catalog snapshot; empty when none do.
pub async fn schema_violations(stub: &Stub) -> Vec<String> {
    calls(stub)
        .await
        .iter()
        .flat_map(|call| catalog::violations(&call["arguments"], call["name"].as_str().unwrap_or("")))
        .collect()
}

pub async fn listings(stub: &Stub) -> usize {
    stub.requests()
        .await
        .iter()
        .filter(|request| method(request).as_deref() == Some("tools/list"))
        .count()
}
