//! Port of `NativeDraftHistoryTests.swift`: a native Seedance draft and its finalize in the account's history
//! (issue #115). `show_generations` echoes `draft` and `draft_job_id` in each job's params.

use higgsfield::mcp::account::MediaRole;
use jutsu_core::{GenerationHistory, HistoryItem, Job, Resolution};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::history::generations_tool;
use super::support::account::full_set;
use super::support::agent_a::{mcp_provider, media, past_generation};
use crate::history_mapping::item;

/// A trimmed `show_generations` page: the paid run's draft (`draft: true`, 480p) and a finalize of it.
const PAGE: &str = r#"
{
  "items": [
    {
      "id": "final-job",
      "type": "video",
      "status": "completed",
      "model": "seedance_2_5",
      "createdAt": 1791200000,
      "liked": false,
      "results": {"rawUrl": "https://example.com/final.mp4"},
      "params": {
        "prompt": "a fox in the snow",
        "resolution": "1080p",
        "draft": false,
        "draft_job_id": "draft-job"
      }
    },
    {
      "id": "draft-job",
      "type": "video",
      "status": "completed",
      "model": "seedance_2_5",
      "createdAt": 1791100000,
      "liked": false,
      "results": {"rawUrl": "https://example.com/draft.mp4"},
      "params": {
        "prompt": "a fox in the snow",
        "mode": "omni_reference",
        "resolution": "480p",
        "aspect_ratio": "9:16",
        "duration": 15,
        "width": 480,
        "height": 854,
        "draft": true,
        "draft_job_id": null,
        "medias": [
          {"role": "image", "data": {"id": "image-1", "type": "media_input", "url": "https://example.com/a.png"}}
        ]
      }
    }
  ],
  "next_cursor": null
}
"#;

async fn history() -> Vec<HistoryItem> {
    let value: Value = serde_json::from_str(PAGE).unwrap();
    let mut tools = full_set();
    tools.push(generations_tool());
    let (provider, _stub) = mcp_provider(tools, move |name, _| {
        if name == "show_generations" {
            value.clone()
        } else {
            json!({})
        }
    })
    .await;
    provider.history(None).await.unwrap().items
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

#[tokio::test]
async fn a_draft_from_history_is_imported_as_a_native_draft() {
    let items = history().await;
    let draft = items.iter().find(|item| item.id == "draft-job").unwrap();
    assert!(draft.is_native_draft && draft.draft_job_id.is_none());

    let job = Job::importing(draft, Some("owner-1".to_owned()), at(1_791_200_000));
    assert!(job.spec.is_draft && job.spec.resolution == Resolution::P480);
    assert_eq!(
        job.remote.as_ref().map(|remote| remote.is_native_draft),
        Some(true)
    );
    // The finalize window counts from here (`Job::finalize_deadline`, tested in Presentation).
    assert_eq!(job.charged_at, Some(at(1_791_100_000)));
}

/// A finalize keeps the draft's job ID; history gives no other link to the draft.
#[tokio::test]
async fn a_finalize_from_history_keeps_its_drafts_id() {
    let items = history().await;
    let finalized = items.iter().find(|item| item.id == "final-job").unwrap();
    assert!(!finalized.is_native_draft && finalized.draft_job_id.as_deref() == Some("draft-job"));

    let job = Job::importing(finalized, Some("owner-1".to_owned()), at(1_791_200_000));
    assert!(!job.spec.is_draft && job.spec.draft_job_id.as_deref() == Some("draft-job"));
    assert_eq!(
        job.remote.as_ref().map(|remote| remote.is_native_draft),
        Some(false)
    );
}

/// Only Seedance 2.5 has the native draft: another model's `draft` isn't read.
#[test]
fn other_models_are_no_native_drafts() {
    let mut generation = past_generation(
        "job-1",
        "video",
        "completed",
        "hf_mult_motion_control",
        Some("https://example.com/out.mp4"),
    );
    generation.media = vec![media(
        MediaRole::Video,
        "video-1",
        Some("https://example.com/in.mp4"),
    )];
    let mut draft = generation;
    draft.is_draft = true;
    assert_eq!(item(&draft).map(|item| item.is_native_draft), Some(false));
}
