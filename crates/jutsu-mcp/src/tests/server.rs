//! JSON-RPC, both eras of the protocol, and the tools as `tools/list` and `tools/call` give them.

use super::{FakeApp, reply_to, request, server};
use crate::server::{PROTOCOL_VERSIONS, tool_result};
use jutsu_agent::{
    AgentBudgetUnit, AgentCapabilities, AgentCommand, AgentError, AgentPage, AgentResponse, AgentSource,
    Budget, CapabilitiesCommand, Connection, ConnectionKind, CreateElementCommand, EnhanceCommand,
    LibrarySearchCommand, ShowCommand, ToolGroup, Uuid, video,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

fn capabilities() -> AgentCapabilities {
    AgentCapabilities::new(
        "0.2.0",
        Connection {
            kind: ConnectionKind::Account,
            plan: Some("Pro".to_owned()),
            credits: Some(1200),
        },
        Some(Budget {
            unit: AgentBudgetUnit::Credits,
            daily_limit: 0,
            spent_today: 0,
            reserved_today: 0,
            remaining: 0,
            unlimited_allowed: false,
            daily_cap: None,
            cap_taken_today: None,
        }),
    )
}

fn app() -> Arc<FakeApp> {
    FakeApp::new(AgentResponse {
        capabilities: Some(capabilities()),
        ..AgentResponse::default()
    })
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "agent-cli", "title": "Agent CLI", "version": "1" },
    })
}

/// One request through a new server.
async fn reply(app: &Arc<FakeApp>, method: &str, params: Option<Value>) -> Value {
    request(&mut server(app, "0.2.0"), method, params, json!(1)).await
}

fn code(answer: &Value) -> Option<i64> {
    answer["error"]["code"].as_i64()
}

// 2026-07-28

#[tokio::test]
async fn discover_describes_the_server_statelessly() {
    let answer = reply(&app(), "server/discover", Some(json!({ "_meta": modern_meta() }))).await;
    let result = &answer["result"];
    assert_eq!(answer["id"], json!(1));
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"][0], "2026-07-28");
    assert!(
        result["supportedVersions"]
            .as_array()
            .expect("versions")
            .contains(&json!("2025-11-25"))
    );
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    assert!(
        result["instructions"]
            .as_str()
            .expect("instructions")
            .contains("Allow agents")
    );
    assert!(result["ttlMs"].as_f64().unwrap_or(0.0) > 0.0);
    assert_eq!(result["cacheScope"], "public");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "jutsu"
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        "0.2.0"
    );
}

#[tokio::test]
async fn an_unsupported_version_is_refused_with_the_supported_ones() {
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "1900-01-01", "io.modelcontextprotocol/clientCapabilities": {},
    });
    let answer = reply(&app(), "tools/list", Some(json!({ "_meta": meta }))).await;
    assert_eq!(code(&answer), Some(-32022));
    assert_eq!(answer["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(answer["error"]["data"]["supported"], json!(PROTOCOL_VERSIONS));
}

#[tokio::test]
async fn a_request_without_the_required_meta_is_invalid_params() {
    let app = app();
    let no_capabilities = json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } });
    assert_eq!(
        code(&reply(&app, "tools/list", Some(no_capabilities)).await),
        Some(-32602)
    );
    let not_a_string = json!({
        "_meta": { "io.modelcontextprotocol/protocolVersion": 7, "io.modelcontextprotocol/clientCapabilities": {} },
    });
    assert_eq!(
        code(&reply(&app, "tools/list", Some(not_a_string)).await),
        Some(-32602)
    );
    // Discover belongs to 2026-07-28 alone, which always names its version.
    assert_eq!(code(&reply(&app, "server/discover", None).await), Some(-32602));
}

#[tokio::test]
async fn modern_tool_calls_carry_result_type_and_the_client_from_meta() {
    let app = app();
    let answer = reply(
        &app,
        "tools/call",
        Some(json!({ "_meta": modern_meta(), "name": "capabilities" })),
    )
    .await;
    let result = &answer["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "jutsu"
    );
    assert_eq!(result["isError"], false);
    let calls = app.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1.as_deref(), Some("Agent CLI"));
}

#[tokio::test]
async fn modern_tool_lists_are_cacheable() {
    let result = reply(&app(), "tools/list", Some(json!({ "_meta": modern_meta() }))).await["result"].clone();
    assert!(result.get("ttlMs").is_some());
    assert_eq!(result["cacheScope"], "public");
    assert_eq!(result["resultType"], "complete");
}

// The handshake revisions

#[tokio::test]
async fn initialize_echoes_a_known_version_and_offers_the_newest_handshake_otherwise() {
    let app = app();
    for version in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let answer = reply(
            &app,
            "initialize",
            Some(json!({ "protocolVersion": version, "capabilities": {} })),
        )
        .await;
        assert_eq!(answer["result"]["protocolVersion"], version);
    }
    for asked in [json!("2026-07-28"), json!("1999-01-01"), json!(3)] {
        let answer = reply(&app, "initialize", Some(json!({ "protocolVersion": asked }))).await;
        assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
    }
    let result = reply(
        &app,
        "initialize",
        Some(json!({ "protocolVersion": "2025-11-25" })),
    )
    .await["result"]
        .clone();
    assert_eq!(result["serverInfo"]["name"], "jutsu");
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": false } })
    );
    assert_ne!(result["instructions"].as_str().expect("instructions"), "");
    assert!(result.get("resultType").is_none());
}

#[tokio::test]
async fn the_handshake_session_names_the_client_and_the_version() {
    let app = app();
    let mut server = server(&app, "0.2.0");
    let params = json!({ "protocolVersion": "2025-03-26", "clientInfo": { "name": "claude-code" } });
    request(&mut server, "initialize", Some(params), json!(1)).await;
    assert_eq!(
        server
            .reply(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await,
        None
    );
    assert_eq!(
        request(&mut server, "ping", None, json!("p")).await["result"],
        json!({})
    );
    // 2025-03-26 has no structured output: no output schemas and no structuredContent.
    let tools = request(&mut server, "tools/list", None, json!(1)).await["result"]["tools"].clone();
    assert!(
        tools
            .as_array()
            .expect("tools")
            .iter()
            .all(|tool| tool.get("outputSchema").is_none())
    );
    let call = request(
        &mut server,
        "tools/call",
        Some(json!({ "name": "capabilities" })),
        json!(1),
    )
    .await;
    assert!(call["result"].get("structuredContent").is_none());
    assert_eq!(call["result"]["content"].as_array().map(Vec::len), Some(1));
    assert_eq!(app.calls()[0].1.as_deref(), Some("claude-code"));
}

// Tools

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one listing checked tool by tool")]
async fn tools_are_listed_in_a_fixed_order_with_schemas_and_hints() {
    let app = app();
    let mut server = server(&app, "0.2.0");
    request(
        &mut server,
        "initialize",
        Some(json!({ "protocolVersion": "2025-11-25" })),
        json!(1),
    )
    .await;
    let tools = request(&mut server, "tools/list", None, json!(1)).await["result"]["tools"].clone();
    let tools = tools.as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        [
            "capabilities",
            "quote",
            "create_video",
            "jobs",
            "job_action",
            "create_image",
            "elements",
            "create_element",
            "library_search",
            "library_get",
            "library_download",
            "library_mark",
            "enhance",
            "delivery_set",
            "export",
            "frames",
            "subtitles",
            "show",
        ]
    );
    let grouped: Vec<String> = ToolGroup::all()
        .into_iter()
        .flat_map(|group| group.tools)
        .collect();
    assert_eq!(names, grouped);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert!(tool["inputSchema"].get("oneOf").is_none());
        assert_eq!(tool["outputSchema"]["type"], "object");
        assert!(tool["outputSchema"]["properties"].get("error").is_some());
        assert!(tool["outputSchema"].get("required").is_none());
        assert!(tool["annotations"].get("readOnlyHint").is_some());
        assert!(tool["annotations"].get("idempotentHint").is_some());
        assert_ne!(tool["description"].as_str().expect("description"), "");
        assert_ne!(tool["title"].as_str().expect("title"), "");
    }
    let by_name = |key: &str| -> HashMap<String, Value> {
        tools
            .iter()
            .map(|tool| {
                (
                    tool["name"].as_str().unwrap_or_default().to_owned(),
                    tool[key].clone(),
                )
            })
            .collect()
    };
    // Image counts are the model's, from capabilities and quote: no fixed maxItems promises another.
    let schemas = by_name("inputSchema");
    for (tool, field) in [
        ("create_video", "images"),
        ("quote", "images"),
        ("create_image", "inputs"),
    ] {
        assert_eq!(schemas[tool]["properties"][field]["type"], "array", "{tool}");
        assert!(
            schemas[tool]["properties"][field].get("maxItems").is_none(),
            "{tool}"
        );
    }
    // The bounds the commands check are the schemas' too.
    let video = &schemas["create_video"]["properties"];
    assert_eq!(video["variants"]["maximum"], json!(video::MAX_VARIANTS));
    assert_eq!(video["length"]["maximum"].as_f64(), Some(video::MAX_LENGTH));
    assert_eq!(
        video["source"]["properties"]["end"]["maximum"].as_f64(),
        Some(AgentSource::MAX_SECONDS)
    );
    assert_eq!(video["prompt"]["maxLength"], json!(video::MAX_PROMPT_LENGTH));
    assert_eq!(
        schemas["quote"]["properties"]["image_count"]["maximum"],
        json!(video::PROTOCOL_MAX_IMAGES)
    );
    let search = &schemas["library_search"]["properties"];
    assert_eq!(
        search["model"]["maxLength"],
        json!(LibrarySearchCommand::MAX_MODEL_LENGTH)
    );
    let element = &schemas["create_element"]["properties"];
    assert_eq!(
        element["name"]["maxLength"],
        json!(CreateElementCommand::MAX_NAME_LENGTH)
    );
    assert_eq!(
        element["images"]["maxItems"],
        json!(CreateElementCommand::MAX_IMAGES)
    );
    assert_eq!(
        schemas["enhance"]["properties"]["voice"]["maxLength"],
        json!(EnhanceCommand::MAX_VOICE_LENGTH)
    );
    let hints = by_name("annotations");
    for name in [
        "capabilities",
        "quote",
        "jobs",
        "elements",
        "library_search",
        "library_get",
    ] {
        assert_eq!(hints[name]["readOnlyHint"], true, "{name}");
        assert_eq!(hints[name]["destructiveHint"], false, "{name}");
    }
    assert_eq!(hints["create_video"]["readOnlyHint"], false);
    assert_eq!(hints["create_video"]["idempotentHint"], true);
    assert_eq!(hints["create_video"]["destructiveHint"], false);
    assert_eq!(hints["job_action"]["readOnlyHint"], false);
    assert_eq!(hints["job_action"]["idempotentHint"], false);
    for name in ["create_image", "enhance", "create_element"] {
        assert_eq!(hints[name]["readOnlyHint"], false, "{name}");
        assert_eq!(hints[name]["idempotentHint"], true, "{name}");
        assert_eq!(hints[name]["destructiveHint"], false, "{name}");
    }
    assert_eq!(hints["library_search"]["openWorldHint"], true);
    assert_eq!(hints["library_download"]["readOnlyHint"], false);
    assert_eq!(hints["library_download"]["destructiveHint"], false);
    assert_eq!(hints["library_download"]["idempotentHint"], true);
    assert_eq!(hints["library_mark"]["idempotentHint"], true);
    assert_eq!(hints["library_mark"]["destructiveHint"], false);
    for name in ["delivery_set", "export", "frames"] {
        assert_eq!(hints[name]["destructiveHint"], false, "{name}");
    }
    // compare_source may read a history item's source from Higgsfield.
    assert_eq!(hints["frames"]["openWorldHint"], true);
    assert_eq!(hints["show"]["readOnlyHint"], false);
    assert_eq!(hints["show"]["idempotentHint"], true);
    assert_eq!(hints["show"]["openWorldHint"], false);
    assert_eq!(
        schemas["create_video"]["required"],
        json!(["request_id", "mode", "max_cost"])
    );
    // The same list twice, byte for byte.
    let again = request(&mut server, "tools/list", None, json!(1)).await["result"]["tools"].clone();
    assert_eq!(&again, &Value::Array(tools.clone()));
}

/// The helper adds its own version, and a note to reconnect when it is older than the app it talks to.
#[tokio::test]
async fn capabilities_say_when_the_helper_is_older_than_the_app() {
    let same = reply(&app(), "tools/call", Some(json!({ "name": "capabilities" }))).await;
    let same = &same["result"]["structuredContent"];
    assert!(same["helperVersion"] == "0.2.0" && same.get("helperNote").is_none());

    let mut updated = capabilities();
    "0.2.10".clone_into(&mut updated.app_version);
    let app = FakeApp::new(AgentResponse {
        capabilities: Some(updated),
        ..AgentResponse::default()
    });
    let mut server = server(&app, "0.2.9");
    let older = request(
        &mut server,
        "tools/call",
        Some(json!({ "name": "capabilities" })),
        json!(1),
    )
    .await;
    let older = &older["result"]["structuredContent"];
    assert_eq!(older["helperVersion"], "0.2.9");
    assert!(older["helperNote"].as_str().expect("note").contains("/mcp"));
}

#[tokio::test]
async fn a_tool_result_is_structured_and_the_same_json_as_text() {
    let answer = reply(&app(), "tools/call", Some(json!({ "name": "capabilities" }))).await;
    let result = &answer["result"];
    let structured = &result["structuredContent"];
    assert_eq!(structured["appVersion"], "0.2.0");
    assert_eq!(structured["connection"]["credits"], 1200);
    assert_eq!(structured["budget"]["unit"], "credits");
    let text = result["content"][0]["text"].as_str().expect("text");
    assert_eq!(&serde_json::from_str::<Value>(text).expect("JSON"), structured);
    assert_eq!(result["isError"], false);
}

#[tokio::test]
async fn an_app_error_is_a_tool_error_with_what_to_do_next() {
    let app = app();
    app.set_answer(AgentResponse {
        error: Some(AgentError::agents_off()),
        ..AgentResponse::default()
    });
    let answer = reply(
        &app,
        "tools/call",
        Some(json!({ "name": "show", "arguments": { "page": "queue" } })),
    )
    .await;
    let result = &answer["result"];
    assert_eq!(result["isError"], true);
    let error = &result["structuredContent"]["error"];
    assert_eq!(error["code"], "agents_off");
    assert_eq!(error["retryable"], true);
    assert!(error["next"].as_str().expect("next").contains("Allow agents"));
    assert_eq!(
        app.commands()[0],
        AgentCommand::Show(ShowCommand::page(AgentPage::Queue))
    );
}

#[tokio::test]
async fn bad_arguments_are_tool_errors_and_never_reach_the_app() {
    let app = app();
    let calls = [
        json!({ "name": "show" }),
        json!({ "name": "show", "arguments": { "page": "queue", "job": "x" } }),
        json!({ "name": "show", "arguments": { "page": "jobs" } }),
        json!({ "name": "show", "arguments": { "job": "not-a-uuid" } }),
        json!({ "name": "show", "arguments": { "page": 3 } }),
        json!({ "name": "show", "arguments": { "page": "queue", "extra": true } }),
        json!({ "name": "capabilities", "arguments": { "models": "yes" } }),
        json!({ "name": "capabilities", "arguments": { "model": "../../etc" } }),
    ];
    for params in calls {
        let result = reply(&app, "tools/call", Some(params.clone())).await["result"].clone();
        assert_eq!(result["isError"], true, "{params}");
        assert_eq!(
            result["structuredContent"]["error"]["code"], "bad_request",
            "{params}"
        );
    }
    assert_eq!(app.calls().len(), 0);
}

#[tokio::test]
async fn good_arguments_become_their_commands() {
    let app = app();
    let job = Uuid::new_v4();
    reply(
        &app,
        "tools/call",
        Some(json!({ "name": "show", "arguments": { "job": job.to_string() } })),
    )
    .await;
    reply(
        &app,
        "tools/call",
        Some(json!({ "name": "capabilities", "arguments": { "model": "seedance25" } })),
    )
    .await;
    reply(
        &app,
        "tools/call",
        Some(json!({ "name": "capabilities", "arguments": { "models": true } })),
    )
    .await;
    assert_eq!(
        app.commands(),
        [
            AgentCommand::Show(ShowCommand::job(job)),
            AgentCommand::Capabilities(CapabilitiesCommand {
                models: true,
                model: Some("seedance25".to_owned()),
                ..CapabilitiesCommand::default()
            }),
            AgentCommand::Capabilities(CapabilitiesCommand {
                models: true,
                ..CapabilitiesCommand::default()
            }),
        ]
    );
}

// Malformed messages

#[tokio::test]
async fn malformed_calls_are_json_rpc_errors() {
    let app = app();
    assert_eq!(
        code(&reply(&app, "tools/call", Some(json!({ "name": "delete_all" }))).await),
        Some(-32602)
    );
    assert_eq!(
        code(&reply(&app, "tools/call", Some(json!({}))).await),
        Some(-32602)
    );
    let listed = json!({ "name": "show", "arguments": ["queue"] });
    assert_eq!(code(&reply(&app, "tools/call", Some(listed)).await), Some(-32602));
    assert_eq!(
        code(&reply(&app, "tools/list", Some(json!(["a"]))).await),
        Some(-32602)
    );
    assert_eq!(code(&reply(&app, "resources/list", None).await), Some(-32601));
    assert_eq!(app.calls().len(), 0);
}

#[tokio::test]
async fn lines_that_arent_requests_get_the_right_answer() {
    let app = app();
    let mut server = server(&app, "0.2.0");
    let parse = reply_to(&mut server, b"{oops").await;
    assert_eq!(code(&parse), Some(-32700));
    assert_eq!(parse["id"], Value::Null);
    assert_eq!(code(&reply_to(&mut server, b"[1,2]").await), Some(-32600));
    let bad_id = reply_to(&mut server, br#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#).await;
    assert_eq!(code(&bad_id), Some(-32600));
    let no_method = reply_to(&mut server, br#"{"jsonrpc":"2.0","id":4}"#).await;
    assert_eq!(code(&no_method), Some(-32600));
    // Responses and notifications get no answer.
    assert_eq!(
        server.reply(br#"{"jsonrpc":"2.0","id":4,"result":{}}"#).await,
        None
    );
    assert_eq!(
        server
            .reply(br#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#)
            .await,
        None
    );
    // A string id comes back as it was.
    assert_eq!(
        request(&mut server, "ping", None, json!("abc")).await["id"],
        "abc"
    );
}

/// Every answer the app gives reaches the agent as structured content, and an empty one is a lost answer.
#[test]
fn an_answer_without_a_result_is_a_lost_answer() {
    let result = tool_result(&AgentResponse::default(), true);
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["error"]["code"], "no_answer");
}
