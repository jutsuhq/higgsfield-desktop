//! `VideoEditTests`: the operations on a result (`upscale_video`, `reframe`, `remove_background`, `dubbing`,
//! `voice_change` and `list_voices`), listed with their tool definitions from the catalog snapshot of 2026-10-15,
//! and every call checked against its `inputSchema`.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{
    HiggsfieldAccount, VideoEdit, VideoEditInput, VideoEditOperation, VideoEditProvider, Voice,
};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, calls, full_set, schema_violations};
use crate::support::catalog;
use crate::support::mcp::{McpServer, params_tool};
use crate::support::stub::Stub;

const JOB: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";

/// The full set plus the snapshot's tools `names`.
fn with(names: &[&str]) -> Vec<Value> {
    let mut tools = full_set();
    tools.extend(names.iter().map(|name| catalog::tool(name)));
    tools
}

async fn edits(
    tools: Vec<Value>,
    reply: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(tools).call(reply)).await
}

fn upscale_edit() -> VideoEdit {
    VideoEdit {
        operation: VideoEditOperation::Upscale {
            resolution: "4k".into(),
            fps: Some(30),
            provider: VideoEditProvider::Bytedance,
            width: Some(1280),
            height: Some(720),
            preset: Some("aigc".into()),
        },
        input: VideoEditInput::Job(JOB.into()),
    }
}

fn reframing(seconds: f64) -> VideoEditOperation {
    VideoEditOperation::Reframe {
        aspect_ratio: "9:16".into(),
        duration_seconds: seconds,
        resolution: "720p".into(),
    }
}

fn dub() -> VideoEditOperation {
    VideoEditOperation::Dub {
        language: "fra".into(),
    }
}

/// The `params` of each `tools/call`.
async fn params(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .map(|call| call["arguments"]["params"].clone())
        .collect()
}

async fn names(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .map(|call| call["name"].clone())
        .collect()
}

#[tokio::test]
async fn upscale_names_the_video_as_video_id_and_reads_the_job() {
    let (account, stub) = edits(
        with(&["upscale_video"]),
        |_, _| json!({"results": [{"id": "job-9"}], "request_id": "req-9"}),
    )
    .await;

    let generation = account.run_edit(&upscale_edit()).await.unwrap();

    assert_eq!(generation.id, "job-9");
    assert_eq!(generation.request_id.as_deref(), Some("req-9"));
    let calls = calls(&stub).await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["name"], "upscale_video");
    assert_eq!(
        calls[0]["arguments"],
        json!({"params": {
            "video_id": JOB, "provider": "bytedance", "resolution": "4k", "fps": 30, "preset": "aigc",
            "width": 1280, "height": 720,
        }})
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn topaz_sends_neither_frame_rate_nor_size() {
    let (account, stub) = edits(
        with(&["upscale_video"]),
        |_, _| json!({"results": [{"id": "job-2"}]}),
    )
    .await;
    let edit = VideoEdit {
        operation: VideoEditOperation::Upscale {
            resolution: "2160p".into(),
            fps: None,
            provider: VideoEditProvider::Topaz,
            width: None,
            height: None,
            preset: None,
        },
        input: VideoEditInput::Media("5b7d1e90-2a4c-4e8f-9d36-7c1a2b3e4f50".into()),
    };

    account.run_edit(&edit).await.unwrap();

    assert_eq!(
        params(&stub).await,
        [
            json!({"video_id": "5b7d1e90-2a4c-4e8f-9d36-7c1a2b3e4f50", "provider": "topaz", "resolution": "2160p"})
        ]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

/// The operation tools take no `get_cost`, so `generate_video` is asked for the catalog model by the whole seconds;
/// nothing is asked of the operation's own tool.
#[tokio::test]
async fn operations_are_quoted_through_generate_video() {
    let cases = [
        (dub(), "dubbing"),
        (
            VideoEditOperation::ChangeVoice {
                voice_id: "v-1".into(),
                voice_type: "preset".into(),
            },
            "voice_change",
        ),
        (VideoEditOperation::RemoveBackground, "video_background_remover"),
    ];
    for (operation, model) in cases {
        let tools = with(&["upscale_video", "remove_background", "dubbing", "voice_change"]);
        let (account, stub) = edits(
            tools,
            |_, _| json!({"cost": {"credits": 21, "credits_exact": 21}}),
        )
        .await;

        assert_eq!(
            account.operation_cost(&operation, Some(7.04)).await,
            Ok(Some(21.0)),
            "{model}"
        );

        assert_eq!(names(&stub).await, ["generate_video"], "{model}");
        assert_eq!(
            params(&stub).await[0],
            json!({"model": model, "get_cost": true, "duration": 7}),
            "{model}"
        );
        assert_eq!(schema_violations(&stub).await, Vec::<String>::new(), "{model}");
    }
}

/// The live server prices an upscale by its resolution and frame rate, so they go along; the preset doesn't change
/// the price and isn't sent.
#[tokio::test]
async fn a_bytedance_upscale_is_quoted_with_its_resolution_and_frame_rate() {
    let (account, stub) = edits(with(&["upscale_video"]), |_, _| json!({"cost": {"credits": 15}})).await;
    let operation = VideoEditOperation::Upscale {
        resolution: "4k".into(),
        fps: Some(60),
        provider: VideoEditProvider::Bytedance,
        width: Some(1),
        height: Some(1),
        preset: Some("aigc".into()),
    };

    assert_eq!(
        account.operation_cost(&operation, Some(10.0)).await,
        Ok(Some(15.0))
    );
    assert_eq!(
        params(&stub).await[0],
        json!({"model": "bytedance_video_upscale", "get_cost": true, "duration": 10, "resolution": "4k", "fps": 60})
    );
}

/// Topaz has no cost estimation, and a length is needed for the others: no call, no price.
#[tokio::test]
async fn topaz_and_a_missing_length_are_not_quoted() {
    let (account, stub) = edits(with(&["upscale_video", "dubbing"]), |_, _| json!({})).await;
    let topaz = VideoEditOperation::Upscale {
        resolution: "1080p".into(),
        fps: None,
        provider: VideoEditProvider::Topaz,
        width: None,
        height: None,
        preset: None,
    };
    assert_eq!(account.operation_cost(&topaz, Some(10.0)).await, Ok(None));
    assert_eq!(account.operation_cost(&dub(), None).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

/// An answer without a price, like Topaz's "does not support alpha v2 cost estimation", leaves it unknown.
#[tokio::test]
async fn an_answer_without_a_price_is_no_price() {
    let (account, _stub) = edits(
        with(&["dubbing"]),
        |_, _| json!({"error": "dubbing backend request failed (422)"}),
    )
    .await;
    assert_eq!(account.operation_cost(&dub(), Some(5.0)).await, Ok(None));
}

#[tokio::test]
async fn remove_background_names_a_job_as_media_id() {
    let (account, stub) = edits(
        with(&["remove_background"]),
        |_, _| json!({"results": [{"id": "job-3"}]}),
    )
    .await;

    let edit = VideoEdit {
        operation: VideoEditOperation::RemoveBackground,
        input: VideoEditInput::Job(JOB.into()),
    };
    assert_eq!(account.run_edit(&edit).await.unwrap().id, "job-3");
    assert_eq!(
        params(&stub).await,
        [json!({"media_id": JOB, "media_type": "video"})]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn reframe_is_quoted_by_length_and_resolution_alone() {
    let (account, stub) = edits(with(&["reframe"]), |_, arguments| {
        if arguments["params"]["get_cost"] == true {
            json!({"cost": {"credits": 30, "credits_exact": 29.5}})
        } else {
            json!({"results": [{"id": "job-5"}]})
        }
    })
    .await;

    assert_eq!(
        account.operation_cost(&reframing(8.0), None).await,
        Ok(Some(29.5))
    );
    let edit = VideoEdit {
        operation: reframing(8.0),
        input: VideoEditInput::Job(JOB.into()),
    };
    assert_eq!(account.run_edit(&edit).await.unwrap().id, "job-5");

    assert_eq!(
        params(&stub).await,
        [
            json!({"get_cost": true, "duration_seconds": 8, "resolution": "720p"}),
            json!({"medias": [{"role": "video", "value": JOB}], "aspect_ratio": "9:16"}),
        ]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_reframe_over_fifteen_seconds_names_its_length_and_resolution() {
    let (account, stub) = edits(with(&["reframe"]), |_, _| json!({"results": [{"id": "job-6"}]})).await;
    let media = "8e4f2a17-9b3d-4c6a-a1e5-0d2c3b4a5f61";

    let edit = VideoEdit {
        operation: reframing(21.5),
        input: VideoEditInput::Media(media.into()),
    };
    account.run_edit(&edit).await.unwrap();

    assert_eq!(
        params(&stub).await,
        [json!({
            "medias": [{"role": "video", "value": media}], "aspect_ratio": "9:16",
            "duration_seconds": 21.5, "resolution": "720p",
        })]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_reframe_whose_schema_drops_get_cost_is_not_asked() {
    let mut tools = full_set();
    tools.push(params_tool("reframe", &["medias"]));
    let (account, stub) = edits(tools, |_, _| json!({})).await;
    assert_eq!(account.operation_cost(&reframing(8.0), None).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn dubbing_and_voice_change_name_the_video_as_video_id() {
    let (account, stub) = edits(
        with(&["dubbing", "voice_change"]),
        |_, _| json!({"results": [{"id": "job-7"}]}),
    )
    .await;

    account
        .run_edit(&VideoEdit {
            operation: dub(),
            input: VideoEditInput::Job(JOB.into()),
        })
        .await
        .unwrap();
    let voice = VideoEdit {
        operation: VideoEditOperation::ChangeVoice {
            voice_id: "v-1".into(),
            voice_type: "element".into(),
        },
        input: VideoEditInput::Media("a2c4e6f8-1b3d-4f5a-8c7e-9d0b1a2c3e4f".into()),
    };
    account.run_edit(&voice).await.unwrap();

    assert_eq!(names(&stub).await, ["dubbing", "voice_change"]);
    assert_eq!(
        params(&stub).await,
        [
            json!({"video_id": JOB, "target_language": "fra"}),
            json!({"video_id": "a2c4e6f8-1b3d-4f5a-8c7e-9d0b1a2c3e4f", "voice_id": "v-1", "voice_type": "element"}),
        ]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_tool_is_reported_before_any_call() {
    let (account, stub) = edits(full_set(), |_, _| json!({})).await;
    assert_eq!(
        account.check_tool(&upscale_edit().operation).await,
        Err(HiggsfieldError::ToolUnavailable("upscale_video".into()))
    );
    assert_eq!(
        account.check_tool(&dub()).await,
        Err(HiggsfieldError::ToolUnavailable("dubbing".into()))
    );
    assert_eq!(account.operation_cost(&reframing(8.0), None).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn an_answer_without_a_job_is_not_a_refusal_but_a_reason_is() {
    let (unreadable, _stub) = edits(with(&["upscale_video"]), |_, _| json!({"ok": true})).await;
    assert_eq!(
        unreadable.run_edit(&upscale_edit()).await,
        Err(HiggsfieldError::Server { status: 200 })
    );
    let (declined, _stub) = edits(
        with(&["upscale_video"]),
        |_, _| json!({"error": "Video too long"}),
    )
    .await;
    assert_eq!(
        declined.run_edit(&upscale_edit()).await,
        Err(HiggsfieldError::validation("Video too long"))
    );
}

#[tokio::test]
async fn voices_are_read_page_by_page() {
    let (account, stub) = edits(with(&["list_voices"]), |_, arguments| {
        if arguments.get("cursor").is_none() {
            json!({
                "voices": [
                    {
                        "voice_id": "v-1", "voice_type": "preset", "name": "Amber", "gender": "female",
                        "preview_url": "https://cdn.example/amber.mp3", "logo_url": "https://cdn.example/amber.png",
                    },
                    {"voice_id": "v-2", "voice_type": "element", "name": "Mine", "gender": null},
                    {"name": "No ID"},
                ],
                "has_more": true, "next_cursor": "c-2",
            })
        } else {
            json!({
                "voices": [{"voice_id": "v-3", "preview_url": "http://cdn.example/plain.mp3"}],
                "has_more": false, "next_cursor": "c-3",
            })
        }
    })
    .await;

    let first = account.voices(None, 100).await.unwrap();
    let second = account.voices(first.next_cursor.as_ref(), 500).await.unwrap();

    let voice = |id: &str, kind: &str, name: &str, preview: Option<&str>| Voice {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        preview_url: preview.map(|url| Url::parse(url).unwrap()),
    };
    assert_eq!(
        first.items,
        [
            voice("v-1", "preset", "Amber", Some("https://cdn.example/amber.mp3")),
            voice("v-2", "element", "Mine", None),
        ]
    );
    assert_eq!(first.next_cursor, Some(json!("c-2")));
    // No type reads as built-in, no name as the ID, and only an https sample is kept.
    assert_eq!(second.items, [voice("v-3", "preset", "v-3", None)]);
    // `has_more: false` ends the list though a cursor came with it.
    assert_eq!(second.next_cursor, None);
    let arguments: Vec<Value> = calls(&stub)
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect();
    assert_eq!(
        arguments,
        [json!({"size": 100}), json!({"size": 100, "cursor": "c-2"})]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_voice_list_of_unknown_shape_is_refused() {
    let (account, _stub) = edits(with(&["list_voices"]), |_, _| json!({"ok": true})).await;
    assert!(account.voices(None, 100).await.is_err());
}
