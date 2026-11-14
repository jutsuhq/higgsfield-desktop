//! Port of `MCPProviderGenjutsuTests.swift`: Genjutsu Styles and the Motion Library, as `get_presets` served them
//! on 2026-10-14.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use jutsu_core::{
    ClosedRange, GenerationProvider, JobSpec, Mode, MotionLibrary, MotionPreset, MotionShelf, PortError,
    ProviderError, Resolution, StylePreset,
};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use super::support::account::calls;
use super::support::agent_b::{
    assert_same, called_tool, credits, inputs, provider, provider_tools, reply_with,
};
use super::support::mcp::{McpServer, Reply, tool};
use super::support::provider::make_provider;
use super::support::stub::{Stub, body_json};

fn tools() -> Vec<Value> {
    let mut tools = provider_tools();
    tools.push(tool(
        "get_presets",
        "Search and browse reusable presets",
        &[
            "source",
            "category",
            "preset_id",
            "initial_inputs",
            "cursor",
            "limit",
            "query",
        ],
    ));
    tools.push(tool(
        "execute_preset",
        "Request execution of one published preset",
        &["source", "preset_id", "inputs"],
    ));
    tools
}

fn style_id() -> Uuid {
    Uuid::parse_str("2B5B68C4-9253-4D79-9D05-81E7718D8D7A").unwrap()
}

fn style() -> Value {
    json!({
        "id": "genjutsu-style:2b5b68c4-9253-4d79-9d05-81e7718d8d7a", "name": "Knitted Yarn World",
        "thumbnail_url": "https://static-public-media.higgsfield.ai/restyle-presets/503426a7.webp",
        "preview_url": "https://static-public-media.higgsfield.ai/restyle-presets/5c2f5251.mp4",
    })
}

fn motion() -> Value {
    json!({
        "id": "genjutsu:trending:846e6f7b-53c1-4132-8d0f-ca05bfa826c8", "name": "Studio Duo Exchange",
        "thumbnail_url": "https://cdn.higgsfield.ai/a.webp", "preview_url": "https://cdn.higgsfield.ai/a.mp4",
        "description": "Motion transfer reference for /genjutsu. Driving video: https://example.com/drive.mp4. Use …",
    })
}

fn restyle() -> JobSpec {
    let mut spec = super::support::agent_b::spec(Mode::Restyle);
    spec.preset_id = Some(style_id());
    spec.prompt = "warm".into();
    spec
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

#[tokio::test]
async fn style_tools_add_restyle() {
    let (provider, _stub) = provider(tools(), |_, _| json!({})).await;
    assert_eq!(
        provider.capabilities().await.unwrap().modes,
        BTreeSet::from([
            Mode::MotionTransfer,
            Mode::ObjectSwap,
            Mode::Edit,
            Mode::Extend,
            Mode::Restyle
        ])
    );
}

#[tokio::test]
async fn styles_become_presets_across_pages() {
    let (provider, stub) = provider(tools(), |_, arguments| {
        if arguments.get("cursor").is_none() {
            json!({"items": [style(), motion()], "next_cursor": 1050})
        } else {
            json!({"items": [], "next_cursor": 2050})
        }
    })
    .await;
    let presets = provider.presets().await.unwrap();
    assert_eq!(
        presets,
        [StylePreset {
            id: style_id(),
            name: "Knitted Yarn World".into(),
            preview_url: url("https://static-public-media.higgsfield.ai/restyle-presets/503426a7.webp"),
            preview_video_url: Some(url(
                "https://static-public-media.higgsfield.ai/restyle-presets/5c2f5251.mp4"
            )),
        }]
    );
    let categories: Vec<Value> = calls(&stub)
        .await
        .into_iter()
        .map(|call| call["arguments"]["category"].clone())
        .collect();
    assert_eq!(categories, [json!("genjutsu-styles"), json!("genjutsu-styles")]);
}

#[tokio::test]
async fn restyle_estimate_follows_the_quotes() {
    let (provider, _stub) = provider(tools(), |_, _| json!({})).await;
    let mut spec = restyle();
    spec.source.as_mut().unwrap().range = ClosedRange::new(0.0, 8.0);
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("58", true));
    spec.resolution = Resolution::P480;
    assert_eq!(provider.estimate(&spec).await.unwrap(), credits("26", true));
}

#[tokio::test]
async fn pathological_preset_paging_fails_instead_of_looping() {
    for repeated_cursor in [true, false] {
        let answering = McpServer::new(tools());
        let earlier = AtomicUsize::new(0);
        let stub = Stub::replying(move |request| {
            // How many requests went to the same URL before this one, as the Swift stub counts them.
            let earlier = earlier.fetch_add(1, Ordering::SeqCst);
            if called_tool(request).as_deref() != Some("get_presets")
                || body_json(request).get("id").is_none()
            {
                return answering.respond(request);
            }
            // A finite stub keeps the pre-fix regression from looping forever.
            if earlier >= 60 {
                return Reply::status(502);
            }
            let cursor = if repeated_cursor { 1050 } else { earlier };
            reply_with(
                request,
                &json!({"result": {"structuredContent": {"items": [style()], "next_cursor": cursor}}}),
            )
        })
        .await;
        let provider = make_provider(&stub, false);
        assert_eq!(
            provider.presets().await.unwrap_err(),
            PortError::Provider(ProviderError::Server { status: 200 }),
            "{repeated_cursor}"
        );
        assert_eq!(
            calls(&stub).await.len(),
            if repeated_cursor { 2 } else { 50 },
            "{repeated_cursor}"
        );
    }
}

#[tokio::test]
async fn restyle_runs_the_style_with_the_uploaded_files() {
    let (provider, stub) = provider(tools(), |name, _| {
        if name == "execute_preset" {
            json!({"job_ids": ["job-9"]})
        } else {
            json!({"execution": {"cost_credits": 57.5}})
        }
    })
    .await;
    assert_eq!(
        provider.server_estimate(&restyle(), &inputs()).await.unwrap(),
        Some(credits("57.5", false))
    );
    let job = provider
        .submit(&restyle(), &inputs(), Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(job.request_id, "job-9");
    let calls = calls(&stub).await;
    let run = calls.last().unwrap();
    assert_eq!(run["name"], "execute_preset");
    assert_same(
        &run["arguments"],
        &json!({
            "source": "genjutsu", "preset_id": "genjutsu-style:2b5b68c4-9253-4d79-9d05-81e7718d8d7a",
            "inputs": {"video": ["video-1"], "references": ["image-1"], "resolution": "720p", "prompt": "warm"},
        }),
    );
}

#[tokio::test]
async fn no_quote_leaves_the_estimate() {
    let (provider, _stub) = provider(tools(), |_, _| json!({"execution": {"available": true}})).await;
    assert_eq!(
        provider.server_estimate(&restyle(), &inputs()).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn motion_library_keeps_entries_with_a_driving_video() {
    let (provider, stub) = provider(
        tools(),
        |_, _| json!({"items": [motion(), style()], "next_cursor": 1003}),
    )
    .await;
    let page = provider
        .motion_presets(MotionShelf::Trending, None)
        .await
        .unwrap();
    assert_eq!(
        page.items,
        [MotionPreset {
            id: "genjutsu:trending:846e6f7b-53c1-4132-8d0f-ca05bfa826c8".into(),
            name: "Studio Duo Exchange".into(),
            video_url: url("https://example.com/drive.mp4"),
            thumbnail_url: Some(url("https://cdn.higgsfield.ai/a.webp")),
            preview_url: Some(url("https://cdn.higgsfield.ai/a.mp4")),
            example_mode: Some(Mode::MotionTransfer),
        }]
    );
    let next = page.next_cursor.unwrap();
    provider
        .motion_presets(MotionShelf::Trending, Some(next))
        .await
        .unwrap();
    let calls = calls(&stub).await;
    assert_same(&calls.last().unwrap()["arguments"]["cursor"], &json!(1003));
    assert_eq!(calls[0]["arguments"]["category"], "genjutsu-trending");
}
