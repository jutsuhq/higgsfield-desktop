//! `VideoVariantsTests`: `generate_video`'s `count` (catalog snapshot of 2026-10-15), 1–4 variants of the same
//! request, each its own job in `results`, capped to 1 by the server with `use_unlim`.

use higgsfield::mcp::account::{MediaRole, VideoMedia, VideoRequest};
use serde_json::{Value, json};

use crate::support::account::{answering, calls};
use crate::support::stub::Stub;

fn request(count: i64, uses_unlimited: bool) -> VideoRequest {
    let media = vec![
        VideoMedia::new(MediaRole::Video, "video-1"),
        VideoMedia::new(MediaRole::Image, "image-1"),
    ];
    let mut request = VideoRequest::new(VideoRequest::KLING_MOTION_CONTROL, media);
    request.prompt = "slow".into();
    request.mode = Some(VideoRequest::KLING_PRO.into());
    request.duration = Some(10.0);
    request.count = count;
    request.uses_unlimited = uses_unlimited;
    request
}

async fn sent_params(stub: &Stub) -> Value {
    calls(stub).await[0]["arguments"]["params"].clone()
}

/// The server caps an unlimited request to one generation, so the count isn't asked for at all.
#[tokio::test]
async fn count_is_sent_only_above_one_and_never_with_unlimited() {
    for (count, uses_unlimited, sent) in [(3, false, Some(json!(3))), (1, false, None), (4, true, None)] {
        let (account, stub) = answering(|_, _| json!({"results": [{"id": "job-1"}]})).await;
        account.generate(&request(count, uses_unlimited)).await.unwrap();
        let params = sent_params(&stub).await;
        assert_eq!(params.get("count").cloned(), sent, "{count} {uses_unlimited}");
        assert_eq!(params["use_unlim"], uses_unlimited);
    }
}

#[tokio::test]
async fn every_job_of_the_answer_is_kept() {
    let (account, _stub) = answering(|_, _| {
        json!({
            "results": [{"id": "job-1"}, {"id": "job-2"}, {"status": "queued"}, {"id": "job-2"}, {"id": "job-3"}],
            "request_id": "trace-1",
        })
    })
    .await;
    let generation = account.generate(&request(4, false)).await.unwrap();
    assert_eq!(generation.id, "job-1");
    assert_eq!(generation.variant_ids, ["job-2", "job-3"]);
    assert_eq!(generation.request_id.as_deref(), Some("trace-1"));
}
