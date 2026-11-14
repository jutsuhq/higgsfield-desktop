//! The network half of `GenjutsuCatalogTests`: the Genjutsu gallery and Genjutsu Styles.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{Generation, GenjutsuCategory, HiggsfieldAccount, StyleRun};
use serde_json::{Value, json};

use crate::support::account::{account, calls, full_set};
use crate::support::mcp::{McpServer, tool};
use crate::support::stub::Stub;

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.extend([
        tool(
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
        ),
        tool(
            "execute_preset",
            "Request execution of one published preset",
            &["source", "preset_id", "inputs"],
        ),
    ]);
    tools
}

async fn genjutsu(call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(tools()).call(call)).await
}

fn motion() -> Value {
    json!({
        "id": "genjutsu:27182599-2cee-4b27-be1a-d13e9d713d37", "name": "Countryside Duo", "kind": "genjutsu-motion",
        "thumbnail_url": "https://cdn.higgsfield.ai/higgsfield_multiplier_video_explore_variant/54b6f52b.webp",
        "preview_url": "https://cdn.higgsfield.ai/genjutsu/preview/genjutsu-h264-600-v1/90d01517.mp4",
        "description": "Object replacement reference for /genjutsu. Driving video: https://d2ol7oe51mr4n9.cloudfront.net/content_user_id/baed0aeb.mp4. Use the user's reference image, never the catalog example's character.",
    })
}

fn run() -> StyleRun {
    StyleRun {
        style_id: "genjutsu-style:2b5b68c4-9253-4d79-9d05-81e7718d8d7a".into(),
        video_id: "video-1".into(),
        image_ids: vec!["image-1".into()],
        prompt: "warm light".into(),
        resolution: "720p".into(),
    }
}

/// `run().inputs`, as the style's `inputs` and `initial_inputs` carry it.
fn inputs() -> Value {
    json!({"video": ["video-1"], "references": ["image-1"], "resolution": "720p", "prompt": "warm light"})
}

#[tokio::test]
async fn gallery_page_asks_for_the_category() {
    let (account, stub) = genjutsu(|_, _| json!({"items": [motion()], "next_cursor": 1003})).await;
    let page = account
        .genjutsu_presets(GenjutsuCategory::Trending, Some(&json!(3)), 99)
        .await
        .unwrap();
    let names: Vec<_> = page.items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["Countryside Duo"]);
    assert_eq!(page.next_cursor, Some(json!(1003)));
    assert_eq!(
        calls(&stub).await[0]["arguments"],
        json!({"source": "genjutsu", "category": "genjutsu-trending", "limit": 50, "cursor": 3})
    );
}

#[tokio::test]
async fn styles_need_their_tools() {
    let (account, _stub) = genjutsu(|_, _| json!({})).await;
    account.refresh_tools().await.unwrap();
    assert!(account.can_run_styles());
    let (plain, _plain_stub) = crate::support::account::account(McpServer::new(full_set())).await;
    plain.refresh_tools().await.unwrap();
    assert!(!plain.can_run_styles());
}

#[tokio::test]
async fn run_sends_the_style_and_inputs() {
    let (account, stub) = genjutsu(|_, _| json!({"job_ids": ["job-1"]})).await;
    let expected = Generation {
        id: "job-1".into(),
        request_id: None,
        credits: None,
        variant_ids: Vec::new(),
    };
    assert_eq!(account.run_style(&run()).await, Ok(expected));
    let call = &calls(&stub).await[0];
    assert_eq!(call["name"], "execute_preset");
    assert_eq!(call["arguments"]["preset_id"], run().style_id.as_str());
    assert_eq!(call["arguments"]["inputs"], inputs());
}

#[tokio::test]
async fn an_unreadable_run_reply_is_not_a_refusal() {
    let (unreadable, _unreadable_stub) = genjutsu(|_, _| json!({"status": "ok"})).await;
    assert_eq!(
        unreadable.run_style(&run()).await,
        Err(HiggsfieldError::Server { status: 200 })
    );
    let (declined, _declined_stub) = genjutsu(|_, _| json!({"error": "Video too short"})).await;
    assert_eq!(
        declined.run_style(&run()).await,
        Err(HiggsfieldError::validation("Video too short"))
    );
}

#[tokio::test]
async fn cost_opens_the_filled_detail() {
    let (account, stub) = genjutsu(|_, _| json!({"execution": {"cost_credits": 57.5}})).await;
    assert_eq!(account.style_cost(&run()).await, Ok(Some(57.5)));
    let arguments = calls(&stub).await[0]["arguments"].clone();
    assert_eq!(arguments["initial_inputs"], inputs());
    assert_eq!(arguments["category"], "all");
}
