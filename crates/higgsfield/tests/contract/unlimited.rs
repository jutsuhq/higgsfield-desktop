//! `UnlimitedAllowanceTests`: `models_explore` `list` pages carry `supports_unlim` on each model item and a
//! top-level `unlim` block for the account. `CatalogSnapshot` has the live shape; anything else reads as "no
//! allowance".

use std::collections::HashSet;

use higgsfield::mcp::account::{MediaRole, UnlimitedAllowance, VideoMedia, VideoRequest};
use serde_json::{Value, json};

use crate::support::account::{account, answering, calls, calls_of};
use crate::support::catalog;
use crate::support::mcp::{McpServer, tool};

fn page() -> Value {
    json!({
        "models": [
            {"id": "hf_mult_motion_control", "supports_unlim": true},
            {"id": "seedance_2_5", "supports_unlim": false},
            {"id": "kling3_0_motion_control"},
        ],
        "unlim": {"available": true},
    })
}

/// The snapshot's `unlim` block, as it would read with an unmetered allowance.
fn unmetered() -> Value {
    json!({"available": true, "expires_at": "2026-10-18T00:00:00Z", "remaining": null})
}

fn allowance(pages: &[Value]) -> UnlimitedAllowance {
    UnlimitedAllowance::from_pages(pages)
}

fn supported(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// The account of the snapshot had no allowance; Seedance 2.0 is the page's one model that takes it.
#[test]
fn the_snapshots_list_page_is_read() {
    let page = catalog::value(catalog::VIDEO_LIST, &[]);
    assert_eq!(
        allowance(&[page]),
        UnlimitedAllowance {
            is_available: false,
            supported_models: supported(&["seedance_2_0"])
        }
    );
    let unmetered = catalog::value(catalog::VIDEO_LIST, &[("unlim", unmetered())]);
    assert!(allowance(&[unmetered]).is_available);
}

/// `get` answers carry the same keys: Kling 3.0 Motion Control takes the allowance, Seedance 2.5 doesn't.
#[test]
fn the_snapshots_models_are_read_as_items() {
    let page = json!({
        "items": [catalog::value(catalog::KLING, &[]), catalog::value(catalog::SEEDANCE, &[])],
        "unlim": unmetered(),
    });
    let read = allowance(&[page]);
    assert!(read.covers(VideoRequest::KLING_MOTION_CONTROL));
    assert!(!read.covers(VideoRequest::SEEDANCE_25));
}

#[test]
fn only_both_signals_cover_and_only_the_supported_model() {
    let read = allowance(&[page()]);
    assert!(read.is_available);
    assert_eq!(read.supported_models, supported(&["hf_mult_motion_control"]));
    assert!(read.covers(VideoRequest::MOTION_CONTROL));
    assert!(!read.covers(VideoRequest::SEEDANCE_25));
    assert!(!read.covers(VideoRequest::KLING_MOTION_CONTROL));

    // Either signal alone, or a flag that isn't a bool, covers nothing.
    let one_signal = [
        json!({"models": [{"id": "hf_mult_motion_control"}]}),
        json!({"models": [{"id": "hf_mult_motion_control", "supports_unlim": true}]}),
        json!({"models": [{"id": "hf_mult_motion_control"}], "unlim": true}),
        json!({"models": [{"id": "hf_mult_motion_control", "supports_unlim": "true"}], "unlim": {"available": true}}),
    ];
    for page in one_signal {
        assert!(
            !allowance(std::slice::from_ref(&page)).covers(VideoRequest::MOTION_CONTROL),
            "{page}"
        );
    }
}

/// The block is read through a page, as `UnlimitedAllowance.isAvailable(_:)` is private here.
#[test]
fn the_unlim_block_says_yes_only_in_a_known_shape() {
    let blocks = [
        (json!(true), true),
        (json!(false), false),
        (json!({"available": true}), true),
        (json!({"can_use": true, "remaining": 3}), true),
        (json!({"available": true, "remaining": 0}), false),
        (json!({"enabled": false}), false),
        (json!({"available": true, "active": false}), false),
        (json!({"remaining": 12}), true),
        (
            json!({"available": true, "expires_at": null, "remaining": null}),
            true,
        ),
        (
            json!({"available": false, "expires_at": null, "remaining": null}),
            false,
        ),
        (json!({"remaining": null}), false),
        (json!({"plan": "ultra"}), false),
        (json!({"available": "yes"}), false),
        (json!("true"), false),
        (Value::Null, false),
    ];
    for (block, available) in blocks {
        assert_eq!(
            allowance(&[json!({"models": [], "unlim": block})]).is_available,
            available,
            "{block}"
        );
    }
}

#[test]
fn items_and_ids_are_read_under_their_other_names() {
    let pages = [
        json!({"items": [{"model_id": "seedance_2_5", "supports_unlim": true}], "unlim": {"available": true}}),
        json!({"data": [{"model": "kling3_0_motion_control", "supports_unlim": true}]}),
    ];
    let read = allowance(&pages);
    assert!(read.covers(VideoRequest::SEEDANCE_25));
    assert!(read.covers(VideoRequest::KLING_MOTION_CONTROL));
}

#[test]
fn use_unlim_is_false_unless_the_request_asks_for_it() {
    let mut request = VideoRequest::new(VideoRequest::MOTION_CONTROL, Vec::new());
    assert_eq!(request.arguments(false).unwrap()["params"]["use_unlim"], false);
    request.uses_unlimited = true;
    assert_eq!(request.arguments(false).unwrap()["params"]["use_unlim"], true);
    assert_eq!(request.arguments(true).unwrap()["params"]["use_unlim"], true);
}

fn explore() -> Value {
    tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "after", "limit"],
    )
}

/// The models the app runs on an account, as it passes them.
fn models() -> HashSet<String> {
    supported(&[
        VideoRequest::MOTION_CONTROL,
        VideoRequest::REPLACE_OBJECT,
        VideoRequest::SEEDANCE_25,
        VideoRequest::KLING_MOTION_CONTROL,
    ])
}

async fn list_calls(stub: &crate::support::stub::Stub) -> Vec<Value> {
    calls_of(stub, "models_explore")
        .await
        .into_iter()
        .map(|call| call["arguments"].clone())
        .collect()
}

#[tokio::test]
async fn without_models_explore_there_is_no_answer() {
    let generate = tool(
        "generate_video",
        "Generate one direct video request",
        &["params", "context"],
    );
    let (account, stub) = account(McpServer::new(vec![generate])).await;
    assert_eq!(account.unlimited_allowance(&models()).await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

/// The snapshot's first page (with an allowance), then a page with the app's models, as `get` gave them.
#[tokio::test]
async fn reads_the_video_list_until_every_model_was_seen() {
    let first = catalog::value(catalog::VIDEO_LIST, &[("unlim", unmetered())]);
    let second = json!({
        "items": [
            catalog::value(catalog::KLING, &[]),
            catalog::value(catalog::SEEDANCE, &[]),
            catalog::value(catalog::MOTION_CONTROL, &[]),
            {"id": "hf_mult_replace_object"},
        ],
        "has_more": true, "next_page_token": "40", "unlim": unmetered(),
    });
    let (account, stub) = account(McpServer::new(vec![explore()]).call(move |_, arguments| {
        if arguments.get("after").is_none() {
            first.clone()
        } else {
            second.clone()
        }
    }))
    .await;

    let read = account.unlimited_allowance(&models()).await.unwrap().unwrap();

    assert!(read.covers(VideoRequest::KLING_MOTION_CONTROL));
    assert!(!read.covers(VideoRequest::SEEDANCE_25) && !read.covers(VideoRequest::MOTION_CONTROL));
    // Every model was seen on page 2, so page 3 isn't read.
    assert_eq!(
        list_calls(&stub).await,
        [
            json!({"action": "list", "type": "video", "limit": 100}),
            json!({"action": "list", "type": "video", "limit": 100, "after": "20"}),
        ]
    );
}

/// The snapshot's page: no allowance now, so the next page isn't read.
#[tokio::test]
async fn no_allowance_on_the_first_page_ends_the_read() {
    let page = catalog::value(catalog::VIDEO_LIST, &[]);
    let (account, stub) = account(McpServer::new(vec![explore()]).call(move |_, _| page.clone())).await;
    assert_eq!(
        account.unlimited_allowance(&models()).await,
        Ok(Some(UnlimitedAllowance {
            is_available: false,
            supported_models: supported(&["seedance_2_0"])
        }))
    );
    assert_eq!(list_calls(&stub).await.len(), 1);
}

#[tokio::test]
async fn a_last_page_ends_the_read() {
    let page = catalog::value(
        catalog::VIDEO_LIST,
        &[("unlim", unmetered()), ("has_more", json!(false))],
    );
    let (account, stub) = account(McpServer::new(vec![explore()]).call(move |_, _| page.clone())).await;
    assert!(
        account
            .unlimited_allowance(&models())
            .await
            .unwrap()
            .unwrap()
            .is_available
    );
    assert_eq!(list_calls(&stub).await.len(), 1);
}

fn request() -> VideoRequest {
    let media = vec![
        VideoMedia::new(MediaRole::Video, "video-1"),
        VideoMedia::new(MediaRole::Image, "image-1"),
    ];
    let mut request = VideoRequest::new(VideoRequest::KLING_MOTION_CONTROL, media);
    request.prompt = "slow".into();
    request.mode = Some(VideoRequest::KLING_PRO.into());
    request.duration = Some(10.0);
    request
}

/// `generate_video`'s `outputSchema` sends `cost` only for `get_cost`; one in a submit's answer is still read.
#[tokio::test]
async fn generate_reads_the_credits_the_answer_names() {
    let (account, _stub) = answering(
        |_, _| json!({"results": [{"id": "job-1"}], "cost": {"credits_exact": 12.5, "credits": 13}}),
    )
    .await;
    assert_eq!(account.generate(&request()).await.unwrap().credits, Some(12.5));
}
