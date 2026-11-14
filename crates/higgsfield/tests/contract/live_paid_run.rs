//! `LivePaidRunTests`: the answers of a paid run on 2026-10-15, trimmed and with example URLs, replayed by a stub.

use higgsfield::core::{GenerationStatus, HiggsfieldError};
use higgsfield::mcp::account::{AccountImageRequest, HiggsfieldAccount, MediaRole, VideoMedia, VideoRequest};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, account_replying, calls, full_set};
use crate::support::catalog;
use crate::support::mcp::{McpServer, Reply};
use crate::support::stub::{Stub, body_json};

/// A stub answering every call with the live answer `name`.
async fn replay(name: &str, tools: Vec<Value>) -> (HiggsfieldAccount, Stub) {
    let live = catalog::live_answer(name);
    let mut listed = full_set();
    listed.extend(tools);
    account(McpServer::new(listed).call(move |_, _| live.answer.clone())).await
}

/// A stub answering `tool` with `text` as an `isError` result, and everything else as usual.
async fn refusing(tool: &'static str, text: &str, answer: Value) -> (HiggsfieldAccount, Stub) {
    let answering = McpServer::new(full_set());
    let text = text.to_owned();
    account_replying(move |request| {
        let message = body_json(request);
        match message.get("id") {
            Some(id) if message["params"]["name"] == tool => Reply::json(&json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"isError": true, "content": [{"type": "text", "text": text}], "structuredContent": answer},
            })),
            _ => answering.respond(request),
        }
    })
    .await
}

const RUNNING: [&str; 10] = [
    "e62128d0-f91e-4c5c-aafd-f1be29d8039e",
    "3e91e2e7-caee-482e-9c9e-c0af66cda89d",
    "8c2f6fdf-d06c-4666-bd7b-f6d8c5eafcfc",
    "b2315e67-3ee8-4b8b-9e73-b62860d37ec8",
    "b283e31d-0916-483a-bad2-428e77749cfc",
    "287a7844-446e-4bd3-b484-7b93f2f6a6b7",
    "6051a981-2b0b-4e86-a17b-829fa5a3d4ba",
    "b46fa887-f1b9-4894-90ba-302e1780bf1c",
    "295989fc-675c-4ab1-930e-10a29cd3d349",
    "b9673ee0-417a-48c1-be0e-d6ec347b7f0a",
];

fn running() -> Vec<String> {
    RUNNING.map(str::to_owned).to_vec()
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
fn the_live_paid_calls_keep_to_the_snapshot_schemas() {
    for name in [
        "generate_video_count_2",
        "generate_image_count_2",
        "generate_image_use_unlim_refused",
        "jobs_wait_running",
        "media_import_url",
    ] {
        let live = catalog::live_answer(name);
        assert_eq!(
            catalog::violations(&live.arguments, &live.tool),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

/// `count: 2` answered two `results[]`, each its own job, and was charged per variant.
#[tokio::test]
async fn video_variants_are_one_job_each() {
    let (account, _stub) = replay("generate_video_count_2", Vec::new()).await;
    let mut request = VideoRequest::new(VideoRequest::SEEDANCE_25, Vec::new());
    request.prompt = "a paper boat".into();
    request.resolution = Some("480p".into());
    request.count = 2;

    let generation = account.generate(&request).await.unwrap();

    assert_eq!(generation.id, "e62128d0-f91e-4c5c-aafd-f1be29d8039e");
    assert_eq!(generation.variant_ids, ["3e91e2e7-caee-482e-9c9e-c0af66cda89d"]);
    assert_eq!(generation.credits, None);
}

/// `soul_2` with `count: 2` answered two jobs, each charged 0.12.
#[tokio::test]
async fn image_variants_are_one_job_each() {
    let (account, _stub) = replay("generate_image_count_2", Vec::new()).await;
    let request = AccountImageRequest::new("soul_2", "a paper boat", Some("1:1".into()), 2);

    let jobs = account.generate_images(&request).await.unwrap();

    let ids: Vec<_> = jobs.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "8c2f6fdf-d06c-4666-bd7b-f6d8c5eafcfc",
            "b2315e67-3ee8-4b8b-9e73-b62860d37ec8"
        ]
    );
}

/// While running, a Seedance job is `in_progress` with `type: "image"`; the status alone is read, so it is in
/// progress, and the finished ones are read with their `result_url`.
#[tokio::test]
async fn jobs_wait_while_running() {
    let (account, _stub) = replay("jobs_wait_running", vec![catalog::tool("jobs_wait")]).await;

    let wait = account.wait_for_jobs(&running()).await.unwrap().unwrap();

    assert_eq!(wait.poll_after_seconds, Some(10.0));
    let finished = ["8c2f6fdf", "b2315e67", "b46fa887", "295989fc"];
    for id in RUNNING {
        let status = &wait.statuses[id];
        if finished.iter().any(|prefix| id.starts_with(prefix)) {
            let GenerationStatus::Completed { video_url } = status else {
                panic!("{id} is {status:?}")
            };
            assert_eq!(video_url.host_str(), Some("example.com"));
            assert!(video_url.path().contains(id));
        } else {
            assert_eq!(*status, GenerationStatus::InProgress, "{id}");
        }
    }
}

#[tokio::test]
async fn jobs_wait_once_every_job_is_done() {
    let (account, _stub) = replay("jobs_wait_completed", vec![catalog::tool("jobs_wait")]).await;

    let wait = account.wait_for_jobs(&running()).await.unwrap().unwrap();

    assert_eq!(wait.statuses.len(), 10);
    assert_eq!(wait.poll_after_seconds, None);
    let url = Url::parse("https://example.com/hf_20261015_132300_6051a981-2b0b-4e86-a17b-829fa5a3d4ba.mp4")
        .unwrap();
    assert_eq!(
        wait.statuses["6051a981-2b0b-4e86-a17b-829fa5a3d4ba"],
        GenerationStatus::Completed { video_url: url }
    );
}

#[tokio::test]
async fn media_import_reads_the_media_id() {
    let (account, stub) = replay("media_import_url", vec![catalog::tool("media_import_url")]).await;
    let link = Url::parse("https://example.com/clip.mp4").unwrap();

    assert_eq!(
        account.import_media(&link, "video").await.unwrap(),
        "86198d56-f1e1-4176-b6bf-68a90d545a01"
    );
    assert_eq!(
        calls(&stub).await[0]["arguments"],
        catalog::live_answer("media_import_url").arguments
    );
}

/// A refused unlimited opt-in started nothing, so it is a refusal with its reason, not an unknown outcome.
#[tokio::test]
async fn a_refused_unlimited_opt_in_is_a_refusal() {
    let live = catalog::live_answer("generate_image_use_unlim_refused");
    let text = live.text.clone().unwrap();
    assert!(live.is_error);
    let request = AccountImageRequest::new("soul_2", "a paper boat", Some("1:1".into()), 1);

    let (images, _images_stub) = refusing("generate_image", &text, live.answer.clone()).await;
    assert_eq!(
        images.generate_images(&request).await,
        Err(HiggsfieldError::validation(text.clone()))
    );

    let (videos, _videos_stub) = refusing("generate_video", &text, json!({})).await;
    assert_eq!(
        videos.generate(&kling_request()).await,
        Err(HiggsfieldError::validation(text))
    );
}

/// Any other execution error of a submit may have started a job.
#[tokio::test]
async fn another_submit_error_is_an_unknown_outcome() {
    let (account, _stub) = refusing(
        "generate_video",
        "Error starting generation: upstream timeout",
        json!({}),
    )
    .await;
    assert_eq!(
        account.generate(&kling_request()).await,
        Err(HiggsfieldError::Server { status: 200 })
    );
}
