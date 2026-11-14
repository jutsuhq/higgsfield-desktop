//! `HiggsfieldAccountTests`: the tool contract of video generation, status, balance and upload.

use higgsfield::core::{GenerationStatus, HiggsfieldError, Moderation};
use higgsfield::mcp::account::{
    AccountBalance, Generation, MediaRole, UploadedMedia, VideoMedia, VideoRequest,
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use url::Url;

use crate::support::account::{account, account_on, account_replying, answering, calls, full_set, listings};
use crate::support::mcp::{Locked, McpServer, Reply, method, tool};
use crate::support::stub::{Stub, body_json, header};

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

fn without(name: &str) -> Vec<Value> {
    full_set()
        .into_iter()
        .filter(|tool| tool["name"] != name)
        .collect()
}

fn validation(message: &str) -> HiggsfieldError {
    HiggsfieldError::validation(message)
}

#[tokio::test]
async fn generate_sends_the_params_and_reads_the_job() {
    let (account, stub) =
        answering(|_, _| json!({"results": [{"id": "job-1"}], "request_id": "trace-1"})).await;

    assert!(account.refresh_tools().await.unwrap());
    let generation = account.generate(&request()).await.unwrap();

    let expected = Generation {
        id: "job-1".into(),
        request_id: Some("trace-1".into()),
        credits: None,
        variant_ids: Vec::new(),
    };
    assert_eq!(generation, expected);
    let sent = json!({
        "name": "generate_video",
        "arguments": {"params": {
            "model": "kling3_0_motion_control",
            "medias": [
                {"role": "video_references", "value": "video-1"},
                {"role": "image_references", "value": "image-1"},
            ],
            "mode": "pro", "duration": 10, "prompt": "slow", "use_unlim": false,
        }},
    });
    assert_eq!(calls(&stub).await[0], sent);
}

#[tokio::test]
async fn an_unreadable_generate_reply_is_not_a_refusal() {
    let (unreadable, _stub) = answering(|_, _| json!({"results": []})).await;
    assert_eq!(
        unreadable.generate(&request()).await,
        Err(HiggsfieldError::Server { status: 200 })
    );

    let (declined, _stub) = answering(|_, _| json!({"error": "Prompt is too long"})).await;
    assert_eq!(
        declined.generate(&request()).await,
        Err(validation("Prompt is too long"))
    );
}

/// A reply with only a `notice` (`type` and `message`, as `generate_video`'s output schema has it).
fn notice() -> Value {
    json!({"notice": {"type": "info", "message": "Nothing was submitted."}})
}

#[tokio::test]
async fn a_notice_beside_results_is_the_generation() {
    let (account, stub) = answering(|_, _| {
        let mut answer = notice();
        answer["results"] = json!([{"id": "paid-job"}]);
        answer
    })
    .await;
    assert_eq!(account.generate(&request()).await.unwrap().id, "paid-job");
    assert_eq!(calls(&stub).await.len(), 1);
}

#[tokio::test]
async fn a_notice_alone_submitted_nothing() {
    let (account, stub) = answering(|_, _| notice()).await;
    assert_eq!(
        account.generate(&request()).await,
        Err(validation("Nothing was submitted."))
    );
    assert_eq!(calls(&stub).await.len(), 1);
}

#[tokio::test]
async fn cost_asks_for_a_quote() {
    let (account, stub) = answering(|_, _| json!({"cost": {"credits": 28, "credits_exact": 27.5}})).await;
    assert!((account.cost(&request()).await.unwrap() - 27.5).abs() < f64::EPSILON);
    assert_eq!(calls(&stub).await[0]["arguments"]["params"]["get_cost"], true);
}

#[tokio::test]
async fn missing_tools_are_reported_by_name() {
    let (account, stub) = account(McpServer::new(vec![tool(
        "job_status",
        "Status",
        &["request_id"],
    )]))
    .await;
    assert!(!account.refresh_tools().await.unwrap());
    assert_eq!(
        account.generate(&request()).await,
        Err(HiggsfieldError::ToolUnavailable("generate_video".into()))
    );
    assert_eq!(
        account.status("job-1").await,
        Err(HiggsfieldError::ToolUnavailable("job_status".into()))
    );
    assert_eq!(account.balance().await, Ok(None));
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn status_reads_the_generation() {
    let cases = [
        (json!({"status": "pending"}), GenerationStatus::Queued),
        (json!({"status": "processing"}), GenerationStatus::InProgress),
        (
            json!({"status": "failed", "error": "Bad input"}),
            GenerationStatus::Failed {
                message: Some("Bad input".into()),
            },
        ),
        (
            json!({"status": "ip_detected"}),
            GenerationStatus::Rejected(Moderation::ProtectedContent),
        ),
        (
            json!({"status": "nsfw"}),
            GenerationStatus::Rejected(Moderation::ExplicitContent),
        ),
        (json!({"status": "cancelled"}), GenerationStatus::Canceled),
        (
            json!({"status": "completed", "results": {"rawUrl": "https://cdn.example/out.mp4"}}),
            GenerationStatus::Completed {
                video_url: Url::parse("https://cdn.example/out.mp4").unwrap(),
            },
        ),
    ];
    for (generation, expected) in cases {
        let answer = json!({ "generation": generation });
        let (account, stub) = answering(move |_, _| answer.clone()).await;
        assert_eq!(account.status("job-1").await, Ok(expected), "{generation}");
        assert_eq!(calls(&stub).await[0]["arguments"], json!({"jobId": "job-1"}));
    }
}

#[tokio::test]
async fn unknown_status_keeps_polling() {
    for generation in [json!({"status": "teleporting"}), json!({})] {
        let answer = json!({ "generation": generation });
        let (account, _stub) = answering(move |_, _| answer.clone()).await;
        assert_eq!(account.status("job-1").await, Ok(GenerationStatus::InProgress));
    }
}

#[tokio::test]
async fn answer_without_generation_or_video_is_unexpected() {
    for answer in [
        json!({"error": "Job not found"}),
        json!({"generation": {"status": "completed"}}),
        json!({"generation": {"status": "completed", "results": {"rawUrl": "file:///home/me/Movies/x.mov"}}}),
    ] {
        let (account, _stub) = answering(move |_, _| answer.clone()).await;
        assert_eq!(
            account.status("job-1").await,
            Err(validation("Unexpected result from the job_status tool"))
        );
    }
}

/// Neither a generation nor an error: the job may be paid and running, so it is polled again.
#[tokio::test]
async fn an_empty_answer_is_polled_again() {
    let (account, _stub) = answering(|_, _| json!({})).await;
    assert_eq!(
        account.status("job-1").await,
        Err(HiggsfieldError::Server { status: 200 })
    );
}

#[tokio::test]
async fn tool_added_later_is_found_by_listing_again() {
    let tools = Locked::new(without("generate_video"));
    let listed = tools.clone();
    let server =
        McpServer::changing(move || listed.value()).call(|_, _| json!({"results": [{"id": "job-1"}]}));
    let (account, stub) = account(server).await;
    assert!(!account.refresh_tools().await.unwrap());

    tools.update(|tools| *tools = full_set());
    assert_eq!(account.generate(&request()).await.unwrap().id, "job-1");
    assert_eq!(listings(&stub).await, 2);
}

#[tokio::test]
async fn still_missing_tool_is_unavailable_after_one_more_listing() {
    let (account, stub) = account(McpServer::new(without("generate_video"))).await;
    assert!(!account.refresh_tools().await.unwrap());

    assert_eq!(
        account.generate(&request()).await,
        Err(HiggsfieldError::ToolUnavailable("generate_video".into()))
    );
    assert_eq!(listings(&stub).await, 2);
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn missing_balance_is_listed_again_only_once_the_catalog_is_a_minute_old() {
    let tools = Locked::new(without("balance"));
    let listed = tools.clone();
    let stub = McpServer::changing(move || listed.value())
        .call(|_, _| json!({"credits": 10}))
        .serve()
        .await;
    let now = Locked::new(OffsetDateTime::UNIX_EPOCH);
    let account = account_on(&stub, Some(now.clone()));
    assert_eq!(account.balance().await, Ok(None));
    tools.update(|tools| *tools = full_set());
    now.update(|now| *now += time::Duration::seconds(59));
    assert_eq!(account.balance().await, Ok(None));
    assert_eq!(listings(&stub).await, 1);

    now.update(|now| *now += time::Duration::seconds(1));
    assert_eq!(
        account.balance().await,
        Ok(Some(AccountBalance {
            credits: 10.0,
            plan: None
        }))
    );
    assert_eq!(listings(&stub).await, 2);
}

#[tokio::test]
async fn balance_reads_credits_and_plan() {
    let (account, _stub) =
        answering(|_, _| json!({"credits": 2596.34, "subscription_plan_type": "ultra"})).await;
    let expected = AccountBalance {
        credits: 2596.34,
        plan: Some("ultra".into()),
    };
    assert_eq!(account.balance().await, Ok(Some(expected)));
}

/// A stub whose `PUT`s get `put_status` and whose MCP calls `upload`'s answer for `media_upload`.
async fn upload_stub(upload: Value, put_status: u16) -> Stub {
    let mcp = McpServer::new(full_set()).call(move |name, _| {
        if name == "media_upload" {
            upload.clone()
        } else {
            json!({"results": [{"media_id": "media-1", "status": "uploaded"}]})
        }
    });
    Stub::replying(move |request| {
        if request.method.as_str() == "PUT" {
            Reply::status(put_status)
        } else {
            mcp.respond(request)
        }
    })
    .await
}

fn temporary_file(extension: &str, contents: &str) -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .unwrap();
    std::fs::write(file.path(), contents).unwrap();
    file
}

/// `media_confirm`'s `type` follows the content type: a video, an audio file (a speech recording), else an image.
#[tokio::test]
async fn upload_puts_the_file_and_confirms_its_media_id() {
    for (content_type, extension, kind) in [
        ("image/png", "png", "image"),
        ("video/mp4", "mp4", "video"),
        ("audio/mpeg", "mp3", "audio"),
    ] {
        let upload = json!({"uploads": [{
            "media_id": "media-1", "upload_url": "https://upload.example/in?X-Signature=abc",
            "url": "https://cdn.example/media-1",
        }]});
        let stub = upload_stub(upload, 200).await;
        let account = account_on(&stub, None);
        let file = temporary_file(extension, "file");

        let media = account.upload(file.path(), content_type).await.unwrap();

        let url = Url::parse("https://cdn.example/media-1").unwrap();
        assert_eq!(
            media,
            UploadedMedia {
                id: "media-1".into(),
                url
            }
        );
        let requests = stub.requests().await;
        let put = requests
            .iter()
            .find(|request| request.method.as_str() == "PUT")
            .unwrap();
        assert_eq!(header(put, "Authorization"), None);
        assert_eq!(header(put, "Content-Type"), Some(content_type));
        let last = calls(&stub).await.pop().unwrap();
        assert_eq!(last["arguments"], json!({"media_id": "media-1", "type": kind}));
    }
}

#[tokio::test]
async fn upload_url_that_is_not_https_is_never_opened() {
    let upload = json!({"uploads": [{"media_id": "m", "upload_url": "http://upload.example/x", "url": "https://cdn.example/x"}]});
    let stub = upload_stub(upload, 200).await;
    let file = temporary_file("mp4", "mp4");
    assert_eq!(
        account_on(&stub, None).upload(file.path(), "video/mp4").await,
        Err(validation("Unexpected result from the media_upload tool"))
    );
    assert!(
        !stub
            .requests()
            .await
            .iter()
            .any(|request| request.method.as_str() == "PUT")
    );
}

/// An account whose server answers `answered` with `result`, which doesn't decode.
async fn answering_method(
    answered: &'static str,
    result: Value,
) -> (higgsfield::mcp::account::HiggsfieldAccount, Stub) {
    let mcp = McpServer::new(full_set());
    account_replying(move |request| {
        let message = body_json(request);
        match message.get("id") {
            Some(id) if method(request).as_deref() == Some(answered) => {
                Reply::json(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
            }
            _ => mcp.respond(request),
        }
    })
    .await
}

#[tokio::test]
async fn unreadable_replies_are_server_errors() {
    let (list, _list_stub) = answering_method("tools/list", json!({"tools": "none"})).await;
    assert_eq!(
        list.refresh_tools().await,
        Err(HiggsfieldError::Server { status: 200 })
    );

    let (call, _call_stub) = answering_method("tools/call", json!({"content": "not a list"})).await;
    assert_eq!(call.balance().await, Err(HiggsfieldError::Server { status: 200 }));
}

#[tokio::test]
async fn refused_put_is_a_server_error() {
    let upload = json!({"uploads": [{"media_id": "m", "upload_url": "https://upload.example/x", "url": "https://cdn.example/x"}]});
    let stub = upload_stub(upload, 403).await;
    let file = temporary_file("mp4", "mp4");
    assert_eq!(
        account_on(&stub, None).upload(file.path(), "video/mp4").await,
        Err(HiggsfieldError::Server { status: 403 })
    );
}
