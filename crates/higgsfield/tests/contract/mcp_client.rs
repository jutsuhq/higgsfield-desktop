//! `MCPClientTests`: the MCP session, paging, refresh-on-401 and the stream reads.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::{JsonRpcError, McpClient, McpError};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::support::mcp::{
    Locked, McpServer, Reply, fixture, method, token_provider, token_provider_counting, tool,
};
use crate::support::stub::{Stub, body_json, form, header};

fn tools() -> Vec<Value> {
    (1..=5)
        .map(|n| tool(&format!("tool_{n}"), &format!("Tool {n}"), &["x"]))
        .collect()
}

fn client(stub: &Stub) -> McpClient {
    McpClient::new(fixture::mcp_url(), token_provider("access-1")).with_http_client(stub.client())
}

fn object(value: &Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap()
}

async fn methods(stub: &Stub) -> Vec<String> {
    stub.requests().await.iter().filter_map(method).collect()
}

#[tokio::test]
async fn initializes_once_and_keeps_the_session() {
    let stub = McpServer::new(tools()).page_size(2).serve().await;
    let client = client(&stub);

    let listed = client.list_tools().await.unwrap();
    client
        .call_tool("tool_1", object(&json!({"x": "y"})))
        .await
        .unwrap();

    let names: Vec<_> = listed.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["tool_1", "tool_2", "tool_3", "tool_4", "tool_5"]);
    assert_eq!(
        methods(&stub).await,
        [
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "tools/list",
            "tools/call"
        ]
    );
    let requests = stub.requests().await;
    let initialize = body_json(&requests[0]);
    assert_eq!(initialize["params"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialize["params"]["clientInfo"]["name"], "Jutsu Studio");
    assert_eq!(header(&requests[0], "Mcp-Session-Id"), None);
    for request in &requests {
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(header(request, "Authorization"), Some("Bearer access-1"));
        assert_eq!(
            header(request, "Accept"),
            Some("application/json, text/event-stream")
        );
    }
    for request in &requests[1..] {
        assert_eq!(header(request, "Mcp-Session-Id"), Some("session-1"));
        assert_eq!(header(request, "MCP-Protocol-Version"), Some("2025-06-18"));
    }
    assert_eq!(body_json(&requests[3])["params"]["cursor"], "2");
    assert_eq!(body_json(&requests[5])["params"]["arguments"], json!({"x": "y"}));
}

#[tokio::test]
async fn reads_tool_results_from_an_event_stream() {
    let stub = McpServer::new(tools())
        .call(|name, arguments| json!({"tool": name, "echo": arguments}))
        .serve()
        .await;
    let result = client(&stub)
        .call_tool("tool_2", object(&json!({"a": 1})))
        .await
        .unwrap();
    assert_eq!(result.value(), Some(json!({"tool": "tool_2", "echo": {"a": 1}})));
    assert_eq!(result.text(), "done");
}

#[tokio::test]
async fn unauthorized_refreshes_once_and_retries() {
    let refreshes = Locked::new(0);
    let stub = McpServer::new(tools()).rejected(&["stale"]).serve().await;
    let client = McpClient::new(
        fixture::mcp_url(),
        token_provider_counting("stale", refreshes.clone()),
    )
    .with_http_client(stub.client());

    assert_eq!(client.list_tools().await.unwrap().len(), 5);
    assert_eq!(refreshes.value(), 1);
    let requests = stub.requests().await;
    assert_eq!(header(&requests[0], "Authorization"), Some("Bearer stale"));
    assert!(
        requests[1..]
            .iter()
            .all(|request| header(request, "Authorization") == Some("Bearer fresh"))
    );
}

#[tokio::test]
async fn second_unauthorized_is_reported() {
    let stub = McpServer::new(tools())
        .rejected(&["stale", "fresh"])
        .serve()
        .await;
    let client = McpClient::new(fixture::mcp_url(), token_provider("stale")).with_http_client(stub.client());
    assert_eq!(
        client.list_tools().await,
        Err(HiggsfieldError::Unauthorized.into())
    );
}

#[tokio::test]
async fn failed_handshake_is_retried_without_its_session_id() {
    let answering = McpServer::new(tools());
    let failed = Locked::new(false);
    let stub = Stub::replying(move |request| {
        let method = method(request);
        if method.as_deref() == Some("notifications/initialized")
            && failed.update(|value| !std::mem::replace(value, true))
        {
            return Reply::status(500);
        }
        if method.as_deref() == Some("initialize") && header(request, "Mcp-Session-Id").is_some() {
            return Reply::status(400);
        }
        answering.respond(request)
    })
    .await;
    let client = client(&stub);
    assert_eq!(
        client.list_tools().await,
        Err(HiggsfieldError::Server { status: 500 }.into())
    );
    assert_eq!(client.list_tools().await.unwrap().len(), 5);
    let requests = stub.requests().await;
    assert!(
        requests
            .iter()
            .filter(|request| method(request).as_deref() == Some("initialize"))
            .all(|request| header(request, "Mcp-Session-Id").is_none())
    );
}

#[tokio::test]
async fn stateless_requests_carry_the_negotiated_version() {
    let answering = McpServer::new(tools());
    let stub = Stub::replying(move |request| {
        let mut reply = answering.respond(request);
        reply.remove_header("Mcp-Session-Id");
        reply
    })
    .await;
    assert_eq!(client(&stub).list_tools().await.unwrap().len(), 5);
    assert!(stub.requests().await[1..].iter().all(|request| {
        header(request, "MCP-Protocol-Version") == Some("2025-06-18")
            && header(request, "Mcp-Session-Id").is_none()
    }));
}

#[tokio::test]
async fn incompatible_initialize_result_stops_before_tools_are_called() {
    let answering = McpServer::new(tools());
    let stub = Stub::replying(move |request| {
        let message = body_json(request);
        match message.get("id") {
            Some(id) if message["method"] == "initialize" => {
                Reply::json(&json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2099-01-01"}}))
            }
            _ => answering.respond(request),
        }
    })
    .await;
    assert_eq!(
        client(&stub).list_tools().await,
        Err(HiggsfieldError::Server { status: 200 }.into())
    );
    assert_eq!(stub.requests().await.len(), 1);
}

/// `McpServer` with a new session ID per `initialize`, answering `404` to any session in `expired`.
async fn session_server(expired: Arc<Locked<HashSet<String>>>) -> Stub {
    let answering = McpServer::new(tools());
    let sessions = Locked::new(0);
    Stub::replying(move |request| {
        if method(request).as_deref() == Some("initialize") {
            let number = sessions.update(|count| {
                *count += 1;
                *count
            });
            let mut reply = answering.respond(request);
            reply.set_header("Mcp-Session-Id", &format!("session-{number}"));
            return reply;
        }
        if let Some(session) = header(request, "Mcp-Session-Id")
            && expired.value().contains(session)
        {
            return Reply::not_found();
        }
        answering.respond(request)
    })
    .await
}

#[tokio::test]
async fn expired_session_is_started_again_and_the_call_retried() {
    let expired = Locked::new(HashSet::new());
    let stub = session_server(expired.clone()).await;
    let client = client(&stub);
    assert_eq!(client.list_tools().await.unwrap().len(), 5);

    expired.update(|sessions| *sessions = HashSet::from(["session-1".to_owned()]));
    let result = client.call_tool("tool_1", serde_json::Map::new()).await.unwrap();

    assert_eq!(result.text(), "done");
    assert_eq!(
        methods(&stub).await,
        [
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call",
            "initialize",
            "notifications/initialized",
            "tools/call"
        ]
    );
    let requests = stub.requests().await;
    assert_eq!(header(&requests[4], "Mcp-Session-Id"), None);
    assert_eq!(header(&requests[5], "Mcp-Session-Id"), Some("session-2"));
    assert_eq!(header(&requests[6], "Mcp-Session-Id"), Some("session-2"));
}

#[tokio::test]
async fn second_expired_session_is_reported() {
    let expired = ["session-1", "session-2", "session-3"].map(str::to_owned);
    let stub = session_server(Locked::new(HashSet::from(expired))).await;
    assert_eq!(
        client(&stub).list_tools().await,
        Err(HiggsfieldError::Server { status: 404 }.into())
    );
    assert_eq!(
        methods(&stub)
            .await
            .iter()
            .filter(|method| *method == "initialize")
            .count(),
        2
    );
}

#[tokio::test]
async fn not_found_without_a_session_is_a_server_error() {
    let stub = Stub::replying(|_| Reply::not_found()).await;
    assert_eq!(
        client(&stub).list_tools().await,
        Err(HiggsfieldError::Server { status: 404 }.into())
    );
    assert_eq!(stub.requests().await.len(), 1);
}

#[tokio::test]
async fn malformed_tool_is_left_out() {
    let tools = tools();
    let listed = vec![
        tools[0].clone(),
        json!({"name": "no_schema"}),
        json!({"description": "no name"}),
        tools[1].clone(),
    ];
    let stub = McpServer::new(listed).serve().await;
    let names: Vec<_> = client(&stub)
        .list_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(names, ["tool_1", "tool_2"]);
}

#[tokio::test]
async fn json_rpc_errors_are_thrown() {
    let stub = Stub::replying(|request| match body_json(request).get("id") {
        None => Reply::status(202),
        Some(id) => Reply::json(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}),
        ),
    })
    .await;
    let expected = JsonRpcError {
        code: -32601,
        message: "Method not found".into(),
    };
    assert_eq!(client(&stub).list_tools().await, Err(McpError::JsonRpc(expected)));
}

/// A server whose every `tools/list` page holds one tool and points to `next(page)`.
async fn paging_server(next: impl Fn(usize) -> String + Send + Sync + 'static) -> Stub {
    Stub::replying(move |request| {
        let message = body_json(request);
        let Some(id) = message.get("id") else {
            return Reply::status(202);
        };
        if message["method"] != "tools/list" {
            return Reply::json(
                &json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-06-18"}}),
            );
        }
        let page = message["params"]["cursor"]
            .as_str()
            .and_then(|cursor| cursor.parse().ok())
            .unwrap_or(0);
        let result = json!({"tools": [tool(&format!("tool_{page}"), "Tool", &[])], "nextCursor": next(page)});
        Reply::json(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    })
    .await
}

#[tokio::test]
async fn tool_paging_stops_on_a_repeated_cursor() {
    let stub = paging_server(|_| "again".into()).await;
    assert_eq!(client(&stub).list_tools().await.unwrap().len(), 2);
    assert_eq!(
        methods(&stub)
            .await
            .iter()
            .filter(|method| *method == "tools/list")
            .count(),
        2
    );
}

#[tokio::test]
async fn tool_paging_stops_after_the_maximum_page_count() {
    let stub = paging_server(|page| (page + 1).to_string()).await;
    assert_eq!(
        client(&stub).list_tools().await.unwrap().len(),
        McpClient::MAX_TOOL_PAGES
    );
}

/// The Swift stub keeps the stream open with keep-alive comments; here the answer is held back past the timeout,
/// which the client's whole-request deadline treats the same way.
#[tokio::test]
async fn a_reply_that_never_comes_times_out() {
    let answering = McpServer::new(tools());
    let stub = Stub::replying(move |request| {
        if method(request).as_deref() != Some("tools/call") {
            return answering.respond(request);
        }
        let mut reply = Reply::event_stream(": ping\n\n");
        reply.delay = Some(Duration::from_secs(30));
        reply
    })
    .await;
    let client = client(&stub).with_request_timeout(Duration::from_millis(300));
    assert_eq!(
        client.call_tool("tool_1", serde_json::Map::new()).await,
        Err(HiggsfieldError::Network.into())
    );
}

#[tokio::test]
async fn refresh_grant_keeps_the_old_refresh_token_when_none_is_issued() {
    let stub = Stub::replying(|_| Reply::json_text(r#"{"access_token":"next","expires_in":60}"#)).await;
    let now = OffsetDateTime::from_unix_timestamp(500).unwrap();
    let client = fixture::client(fixture::server(fixture::CLERK_METADATA), stub.client(), now);
    let refreshed = client
        .refresh(&fixture::tokens("access-1", Some("refresh-1"), now))
        .await
        .unwrap();
    assert_eq!(refreshed.access_token, "next");
    assert_eq!(refreshed.refresh_token.as_deref(), Some("refresh-1"));
    assert_eq!(refreshed.expires_at, now + time::Duration::seconds(60));
    let form = form(&stub.requests().await[0]);
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "refresh-1");
    assert_eq!(form["client_id"], "client-1");
    assert_eq!(form["resource"], "https://mcp.higgsfield.ai/mcp");
}
