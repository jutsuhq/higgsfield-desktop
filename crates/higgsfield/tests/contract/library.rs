//! `AccountLibraryTests` and `GenerationsTests`: likes, credit history, uploads and the account's history.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{
    AccountAsset, AssetKind, GenerationType, HiggsfieldAccount, MediaRole, VideoMedia, VideoRequest,
};
use higgsfield::mcp::json::server_media;
use serde_json::{Value, json};
use time::OffsetDateTime;
use url::Url;

use crate::support::account::{account, calls, calls_of, full_set};
use crate::support::mcp::{McpServer, recorded_generations, tool};
use crate::support::stub::{Stub, body_text};

fn library_tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.extend([
        tool(
            "set_generation_like",
            "Like or unlike a generation",
            &["job_id", "liked", "folder_id"],
        ),
        tool(
            "transactions",
            "List the user's credit transactions",
            &["cursor", "size"],
        ),
        tool(
            "assets_picker_list",
            "List existing uploaded assets",
            &["source", "kind", "cursor", "size", "sort", "project_id"],
        ),
    ]);
    tools
}

async fn library(reply: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(library_tools()).call(reply)).await
}

fn date(seconds: f64) -> OffsetDateTime {
    #[allow(clippy::cast_possible_truncation)]
    let nanoseconds = (seconds * 1e9).round() as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds).unwrap()
}

#[tokio::test]
async fn like_sends_the_job_and_the_state() {
    let (account, stub) = library(|_, _| json!({"liked": true})).await;

    account.set_liked("job-1", true).await.unwrap();
    account.set_liked("job-1", false).await.unwrap();

    assert_eq!(
        calls(&stub).await,
        [
            json!({"name": "set_generation_like", "arguments": {"job_id": "job-1", "liked": true}}),
            json!({"name": "set_generation_like", "arguments": {"job_id": "job-1", "liked": false}}),
        ]
    );
}

#[tokio::test]
async fn transactions_read_amounts_and_microsecond_times() {
    let (account, stub) = library(|_, _| {
        json!({
            "next_cursor": 30,
            "items": [
                {
                    "created_at": "2026-10-14T12:25:13.091154Z", "credits": 39, "action": "refund",
                    "display_name": "Higgsfield Genjutsu - Object Swap",
                },
                {"created_at": "2026-10-08T08:55:17Z", "credits": -54.41, "action": "spend"},
                {"credits": -1, "action": "spend"},
            ],
        })
    })
    .await;

    let page = account.transactions(Some(&json!(0)), 500).await.unwrap();

    assert_eq!(page.next_cursor, Some(json!(30)));
    assert_eq!(page.items.len(), 2);
    let refund = &page.items[0];
    assert!((refund.credits - 39.0).abs() < f64::EPSILON && refund.action == "refund");
    assert_eq!(refund.title, "Higgsfield Genjutsu - Object Swap");
    let seconds = (refund.date - OffsetDateTime::UNIX_EPOCH).as_seconds_f64();
    assert!((seconds - 1_791_980_713.091_154).abs() < 0.000_01);
    assert!((page.items[1].credits + 54.41).abs() < f64::EPSILON && page.items[1].title.is_empty());
    assert_eq!(
        calls(&stub).await[0],
        json!({"name": "transactions", "arguments": {"size": 100, "cursor": 0}})
    );
}

#[tokio::test]
async fn uploads_read_videos_with_their_thumbnails() {
    let (account, stub) = library(|_, _| {
        json!({
            "next_cursor": null,
            "items": [
                {
                    "id": "video-1", "kind": "video", "url": "https://cdn.example/video-1.mp4",
                    "thumbnailUrl": "https://cdn.example/video-1_thumb.webp", "width": 1080, "height": 1920,
                    "duration": 8, "createdAt": 1_791_030_465.453_116,
                },
                {"id": "broken", "kind": "video"},
                {"id": "audio-1", "kind": "audio", "url": "https://cdn.example/a.mp3"},
            ],
        })
    })
    .await;

    let page = account.uploads(AssetKind::Video, None, 30).await.unwrap();

    assert_eq!(page.next_cursor, None);
    assert_eq!(page.items.len(), 1);
    let asset = &page.items[0];
    let expected = AccountAsset {
        id: "video-1".into(),
        kind: AssetKind::Video,
        url: Url::parse("https://cdn.example/video-1.mp4").unwrap(),
        thumbnail_url: Some(Url::parse("https://cdn.example/video-1_thumb.webp").unwrap()),
        width: Some(1080),
        height: Some(1920),
        duration: Some(8.0),
        created_at: asset.created_at,
    };
    assert_eq!(*asset, expected);
    assert!((asset.created_at - date(1_791_030_465.453_116)).abs() < time::Duration::microseconds(1));
    assert_eq!(
        calls(&stub).await[0],
        json!({"name": "assets_picker_list", "arguments": {"source": "uploads", "kind": "video", "size": 30}})
    );
}

#[tokio::test]
async fn an_answer_without_items_is_unexpected() {
    let (account, _stub) = library(|_, _| json!({"error": "nope"})).await;
    assert!(account.transactions(None, 100).await.is_err());
    assert!(account.uploads(AssetKind::Image, None, 30).await.is_err());
}

fn kling_request() -> VideoRequest {
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

#[test]
fn kling_background_goes_into_the_params() {
    let mut request = kling_request();
    request.background_source = Some(VideoRequest::BACKGROUND_FROM_VIDEO.into());
    assert_eq!(
        request.arguments(false).unwrap()["params"]["background_source"],
        "input_video"
    );
    assert_eq!(
        kling_request().arguments(false).unwrap()["params"].get("background_source"),
        None
    );
}

/// Left out unless set, under the keys `models_explore get` of `seedance_2_5` names.
#[test]
fn seedance_parameters_go_into_the_params_only_when_set() {
    let mut request = kling_request();
    request.bitrate_mode = Some("high".into());
    request.mode = Some(VideoRequest::SEEDANCE_EXTEND.into());
    request.extension_mode = Some(VideoRequest::EXTEND_FORWARD.into());
    let params = request.arguments(false).unwrap()["params"].clone();
    assert_eq!(params["bitrate_mode"], "high");
    assert_eq!(params["mode"], "video_extension");
    assert_eq!(params["extension_mode"], "forward");
    let unset = kling_request().arguments(false).unwrap()["params"].clone();
    assert!(unset.get("bitrate_mode").is_none() && unset.get("extension_mode").is_none());
}

// History.

async fn history(reply: impl Fn(&Value) -> Value + Send + Sync + 'static) -> (HiggsfieldAccount, Stub) {
    let mut tools = full_set();
    tools.push(tool(
        "show_generations",
        "Show the user's generations",
        &["type", "only_completed", "size", "cursor"],
    ));
    let server = McpServer::new(tools).call(move |name, arguments| {
        if name == "show_generations" {
            reply(arguments)
        } else {
            json!({})
        }
    });
    account(server).await
}

/// Swift's `generations()`: completed videos, 24 a page.
async fn first_page(
    account: &HiggsfieldAccount,
) -> Result<higgsfield::mcp::account::GenerationPage, HiggsfieldError> {
    account
        .generations(Some(GenerationType::Video), true, None, 24)
        .await
}

#[tokio::test]
async fn recorded_page_decodes() {
    let recorded = recorded_generations();
    let (account, stub) = history(move |_| recorded.clone()).await;

    let page = first_page(&account).await.unwrap();

    let item = &page.items[0];
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next_cursor, Some(json!("<cursor; value not recorded>")));
    assert_eq!(item.id, "<job id>");
    assert_eq!(item.kind, "video");
    assert_eq!(item.status, "completed");
    assert_eq!(item.model, VideoRequest::REPLACE_OBJECT);
    assert!((item.created_at - date(1_791_032_606.742_653)).abs() < time::Duration::microseconds(1));
    assert!(!item.is_liked);
    let output = item.output_url.as_ref().unwrap();
    assert_eq!(output.host_str(), Some("d8j0ntlcm91z4.cloudfront.net"));
    assert_eq!(
        std::path::Path::new(output.path()).extension(),
        Some("mp4".as_ref())
    );
    assert!(
        item.media
            .iter()
            .all(|media| media.url.as_ref().and_then(Url::host_str) == Some("d2ol7oe51mr4n9.cloudfront.net"))
    );
    assert_eq!((item.width, item.height), (Some(1080), Some(1920)));
    assert_eq!(item.duration, Some(8.0));
    assert_eq!(item.prompt, "<1712 characters>");
    assert_eq!(item.resolution.as_deref(), Some("720p"));
    let roles: Vec<_> = item.media.iter().map(|media| media.role).collect();
    assert_eq!(
        roles,
        [
            Some(MediaRole::Video),
            Some(MediaRole::Image),
            Some(MediaRole::Image),
            Some(MediaRole::Image)
        ]
    );
    assert!(item.media.iter().all(|media| media.id == "<media id>"));
    assert_eq!(item.mode, None);
    let call = &calls_of(&stub, "show_generations").await[0];
    assert_eq!(
        call["arguments"],
        json!({"size": 24, "only_completed": true, "type": "video"})
    );
}

#[tokio::test]
async fn cursor_goes_back_unchanged() {
    let cursor = json!({"after": "abc", "offset": 24});
    let (account, stub) = history(|_| json!({"items": [], "next_cursor": null})).await;

    let page = account
        .generations(None, false, Some(&cursor), 500)
        .await
        .unwrap();

    assert_eq!(page.items.len(), 0);
    assert_eq!(page.next_cursor, None);
    let call = &calls_of(&stub, "show_generations").await[0];
    assert_eq!(
        call["arguments"],
        json!({"size": 100, "only_completed": false, "cursor": cursor})
    );
}

#[tokio::test]
async fn large_numeric_cursor_goes_back_exactly() {
    for text in [
        "9007199254740993",
        "9223372036854775807",
        "18446744073709551615",
        "-9007199254740993",
    ] {
        let cursor: Value = serde_json::from_str(text).unwrap();
        let (account, stub) = history(move |_| json!({"items": [], "next_cursor": cursor})).await;
        let page = first_page(&account).await.unwrap();
        account
            .generations(Some(GenerationType::Video), true, page.next_cursor.as_ref(), 24)
            .await
            .unwrap();
        let request = body_text(stub.requests().await.last().unwrap());
        assert!(request.contains(&format!("\"cursor\":{text}")), "{request}");
    }
}

#[tokio::test]
async fn invalid_dimensions_are_omitted() {
    for width in [1e30, -1.0, 12.5] {
        let (account, _stub) =
            history(move |_| json!({"items": [{"id": "job", "params": {"width": width, "height": 720}}]}))
                .await;
        let page = first_page(&account).await.unwrap();
        assert_eq!(page.items[0].width, None, "{width}");
        assert_eq!(page.items[0].height, Some(720));
    }
}

#[tokio::test]
async fn items_without_an_id_are_left_out() {
    let (account, _stub) = history(|_| json!({"items": [{"type": "video"}, {"id": "job-2"}]})).await;
    let ids: Vec<_> = first_page(&account)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, ["job-2"]);
}

#[tokio::test]
async fn page_without_items_is_unexpected() {
    let (account, _stub) = history(|_| json!({"error": "nope"})).await;
    assert_eq!(
        first_page(&account).await,
        Err(HiggsfieldError::validation(
            "Unexpected result from the show_generations tool"
        ))
    );
}

#[tokio::test]
async fn missing_tool_is_reported_by_name() {
    let (account, _stub) = account(McpServer::new(full_set())).await;
    assert_eq!(
        first_page(&account).await,
        Err(HiggsfieldError::ToolUnavailable("show_generations".into()))
    );
}

#[test]
fn history_roles_map_to_request_roles() {
    let cases = [
        ("video", Some(MediaRole::Video)),
        ("image", Some(MediaRole::Image)),
        ("video_references", Some(MediaRole::Video)),
        ("image_references", Some(MediaRole::Image)),
        ("start_image", Some(MediaRole::StartFrame)),
        ("end_image", Some(MediaRole::EndFrame)),
        ("audio", Some(MediaRole::Audio)),
        ("audio_references", Some(MediaRole::Audio)),
        ("mask", None),
    ];
    for (role, expected) in cases {
        assert_eq!(MediaRole::from_history_role(role), expected, "{role}");
    }
}

#[test]
fn server_media_urls_must_be_https() {
    let cases = [
        ("https://cdn.example/a b.mp4", true),
        ("HTTPS://cdn.example/a.mp4", true),
        ("http://cdn.example/a.mp4", false),
        ("file:///home/me/Movies/x.mov", false),
        ("ftp://cdn.example/a.mp4", false),
        ("/tmp/a.mp4", false),
        ("https:///a.mp4", false),
    ];
    for (text, accepted) in cases {
        assert_eq!(server_media(text).is_some(), accepted, "{text}");
    }
}
