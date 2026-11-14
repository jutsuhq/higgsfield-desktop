//! `CatalogExportTests` and `CatalogSnapshotTests`: the catalog export, and the schema checks the other tests use.

use std::sync::Arc;

use higgsfield::mcp::McpClient;
use higgsfield::mcp::account::{HiggsfieldAccount, MediaRole, VideoMedia, VideoRequest};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::support::account::{calls, listings};
use crate::support::catalog::{violations, violations_of_params};
use crate::support::mcp::{McpServer, fixture, token_provider, tool};
use crate::support::stub::{Stub, header};

/// A tool with every field `tools/list` may carry, which `McpTool` doesn't decode.
fn described() -> Value {
    json!({
        "name": "job_status", "title": "Job status", "description": "Check a job",
        "annotations": {"readOnlyHint": true},
        "inputSchema": {"type": "object", "properties": {"jobId": {"type": "string"}}},
        "outputSchema": {"type": "object", "properties": {"generation": {"type": "object"}}},
    })
}

fn explore() -> Value {
    tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "after", "limit"],
    )
}

fn date() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_791_000_000).unwrap()
}

async fn export_account(
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    let stub = McpServer::new(tools).page_size(2).call(call).serve().await;
    let client =
        McpClient::new(fixture::mcp_url(), token_provider("access-1")).with_http_client(stub.client());
    let account = HiggsfieldAccount::new(Arc::new(client))
        .with_http_client(stub.client())
        .with_clock(Arc::new(date));
    (account, stub)
}

fn unlim() -> Value {
    json!({"available": false, "expires_at": null, "remaining": null})
}

/// Video models come in two pages; the others in one. Answers carry the account's `unlim` block, as in the
/// catalog snapshot of 2026-10-15.
fn answer(_: &str, arguments: &Value) -> Value {
    match (
        arguments["action"].as_str(),
        arguments["type"].as_str(),
        arguments.get("after"),
    ) {
        (Some("list"), Some("video"), None) => {
            json!({"items": [{"id": "seedance_2_5"}], "has_more": true, "next_page_token": "100", "unlim": unlim()})
        }
        (Some("list"), Some("video"), Some(_)) => {
            json!({"items": [{"id": "kling3_0_motion_control"}], "has_more": false, "unlim": unlim()})
        }
        (Some("list"), Some(kind), _) => {
            json!({"items": [{"id": format!("{kind}-model")}], "has_more": false})
        }
        _ => json!({"id": arguments["model_id"], "supports_unlim": true, "unlim": unlim()}),
    }
}

#[tokio::test]
async fn exports_the_raw_tools_and_explores_the_models() {
    let tools = vec![described(), tool("balance", "Credits", &[]), explore()];
    let (account, stub) = export_account(tools.clone(), answer).await;

    let export: Value = serde_json::from_slice(&account.export_catalog(&[]).await.unwrap()).unwrap();

    assert_eq!(export["tools"], Value::Array(tools));
    assert_eq!(listings(&stub).await, 2);
    assert_eq!(export["exportedAt"], date().format(&Rfc3339).unwrap());
    let lists = &export["models"]["list"];
    assert_eq!(lists["video"].as_array().unwrap().len(), 2);
    let arguments: Vec<_> = lists["video"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["arguments"].clone())
        .collect();
    assert_eq!(
        arguments,
        [
            json!({"action": "list", "type": "video", "limit": 100}),
            json!({"action": "list", "type": "video", "limit": 100, "after": "100"}),
        ]
    );
    assert_eq!(
        lists["image"][0]["structuredContent"]["items"],
        json!([{"id": "image-model"}])
    );
    assert_eq!(lists["audio"].as_array().unwrap().len(), 1);
    let gets = export["models"]["get"].as_object().unwrap();
    // Every listed video and image model is described; audio models aren't.
    let mut described: Vec<_> = gets.keys().map(String::as_str).collect();
    described.sort_unstable();
    assert_eq!(
        described,
        ["image-model", "kling3_0_motion_control", "seedance_2_5"]
    );
    assert_eq!(gets["seedance_2_5"]["structuredContent"]["unlim"], unlim());
    assert_eq!(gets["seedance_2_5"]["structuredContent"]["supports_unlim"], true);
    assert!(
        calls(&stub)
            .await
            .iter()
            .all(|call| call["name"] == "models_explore")
    );
}

#[tokio::test]
async fn the_export_is_pretty_and_key_sorted() {
    let (account, _stub) = export_account(vec![explore()], answer).await;
    let text = String::from_utf8(account.export_catalog(&[]).await.unwrap()).unwrap();
    assert!(text.starts_with("{\n  \"exportedAt\""), "{text}");
    assert!(text.contains("\"models\" : {"));
}

#[tokio::test]
async fn no_token_session_or_email_is_exported() {
    let (account, stub) = export_account(vec![described(), explore()], |name, arguments| {
        let mut answer = answer(name, arguments);
        answer["owner"] = json!("me@example.com");
        answer
    })
    .await;

    let export = account
        .export_catalog(&["me@example.com".to_owned()])
        .await
        .unwrap();
    let text = String::from_utf8(export).unwrap();

    assert!(
        stub.requests()
            .await
            .iter()
            .any(|request| header(request, "Mcp-Session-Id") == Some("session-1"))
    );
    for secret in [
        "access-1",
        "refresh-1",
        "session-1",
        "client-1",
        "me@example.com",
        "Bearer",
    ] {
        assert!(!text.contains(secret), "{secret} is in the export");
    }
    assert!(text.contains("<redacted>"));
}

#[tokio::test]
async fn a_server_without_models_explore_still_exports_its_tools() {
    let (account, stub) = export_account(vec![described()], |_, _| json!({})).await;

    let export: Value = serde_json::from_slice(&account.export_catalog(&[]).await.unwrap()).unwrap();

    assert_eq!(export["tools"], json!([described()]));
    assert_eq!(export["models"], Value::Null);
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[test]
fn the_schema_check_catches_what_the_server_refuses() {
    // Topaz takes no frame rate, and a video is named by a UUID.
    let upscale =
        json!({"params": {"provider": "topaz", "video_id": "job-1", "resolution": "2160p", "fps": 30}});
    assert_ne!(violations(&upscale, "upscale_video"), Vec::<String>::new());
    // The first guess of the Virality Predictor call.
    let virality = json!({"action": "create", "job_id": "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17"});
    assert_ne!(violations(&virality, "virality_predictor"), Vec::<String>::new());
    // A language outside the enum.
    let dub =
        json!({"params": {"video_id": "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17", "target_language": "nld"}});
    assert_ne!(violations(&dub, "dubbing"), Vec::<String>::new());
    let french =
        json!({"params": {"video_id": "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17", "target_language": "fra"}});
    assert_eq!(violations(&french, "dubbing"), Vec::<String>::new());
}

/// The model schemas of `models_explore get`: roles, parameter names, options, ranges and types.
#[test]
fn the_model_schema_check_catches_what_the_model_doesnt_take() {
    let media = vec![
        VideoMedia::new(MediaRole::Video, "video-1"),
        VideoMedia::new(MediaRole::Image, "image-1"),
    ];
    let mut kling = VideoRequest::new("kling_video_edit", media);
    kling.prompt = "make it rain".into();
    kling.mode = Some(VideoRequest::KLING_PRO.into());
    assert_eq!(
        violations_of_params(&kling.arguments(false).unwrap()["params"]),
        Vec::<String>::new()
    );

    let mut wan = VideoRequest::new("wan2_7", vec![VideoMedia::new(MediaRole::Video, "video-1")]);
    wan.resolution = Some("480p".into());
    wan.duration = Some(30.0);
    wan.generate_audio = Some(true);
    wan.aspect_ratio = Some("auto".into());
    let found = violations_of_params(&wan.arguments(false).unwrap()["params"]);
    // Wan 2.7 takes no reference video, no "auto" frame, at most 15 s, no soundtrack switch and no 480p.
    let expected = [
        "wan2_7: role video_references",
        "wan2_7: aspect_ratio",
        "wan2_7.duration: above max",
        "wan2_7: generate_audio isn't a parameter",
        "wan2_7.resolution:",
    ];
    assert_eq!(found.len(), expected.len(), "{found:?}");
    for prefix in expected {
        assert!(found.iter().any(|found| found.starts_with(prefix)), "{prefix}");
    }
    assert!(std::panic::catch_unwind(|| violations_of_params(&json!({"model": "no_such_model"}))).is_err());
}
