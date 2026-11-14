//! `QueuedNoticeTests`, the `SupportIDs` and `SubmitDecision.budgeted` rules, and how refusals read.

use jutsu_core::{FailureReason, JobState, RemoteJob};

use super::*;
use crate::queue::fixture::{job, now, usd};

#[test]
fn the_notice_names_what_was_queued_and_its_price() {
    let mut draft = job(JobState::Preparing);
    draft.spec.is_draft = true;
    assert_eq!(
        queued_notice(std::slice::from_ref(&draft)).as_deref(),
        Some("Draft queued · $8.17")
    );
    let mut estimate = job(JobState::Preparing);
    estimate.cost = Cost::usd(Decimal::new(68, 2), true);
    assert_eq!(
        queued_notice(&[estimate]).as_deref(),
        Some("Final queued · ~$0.68")
    );
    assert_eq!(
        queued_notice(&[draft.clone(), draft]).as_deref(),
        Some("2 jobs queued · $16.34")
    );
    let mut unknown = job(JobState::Preparing);
    unknown.cost = Cost::UNKNOWN_USD;
    assert_eq!(
        queued_notice(&[unknown]).as_deref(),
        Some("Final queued · Price unknown")
    );
    assert_eq!(queued_notice(&[]), None);
}

#[test]
fn support_ids_only_for_failed_or_rejected_jobs_the_api_knows() {
    let mut failed = job(JobState::Failed {
        reason: FailureReason::GenerationFailed,
    });
    assert_eq!(support_ids(&failed), None);
    failed.remote = Some(RemoteJob::new("req-1"));
    let line = support_ids(&failed).unwrap();
    assert!(line.starts_with("request_id=req-1 correlation_id=none app="));
    assert!(line.contains(" build=none os="));
    let mut done = job(JobState::Done);
    done.remote = Some(RemoteJob::new("req-2"));
    assert_eq!(support_ids(&done), None);
}

#[test]
fn going_over_the_cap_asks_first() {
    let cap = SpendCap::usd(Decimal::from(10));
    assert_eq!(budgeted(1, Some(&usd("2")), Decimal::from(5), &cap), None);
    let over = budgeted(1, Some(&usd("8.168")), Decimal::from(5), &cap).unwrap();
    assert_eq!(over.title, "Over today's spend cap");
    assert_eq!(
        over.message,
        "This job adds $8.17, bringing today to $13.17 against your $10.00 cap."
    );
    assert_eq!(over.confirm_title, "Start anyway");
    let unknown = budgeted(3, None, Decimal::from(5), &cap).unwrap();
    assert_eq!(unknown.title, "Price not known yet");
    assert_eq!(
        unknown.message,
        "These 3 jobs' price isn't known, so it may take today past your $10.00 cap. Today is at $5.00 so far."
    );
    let credits = SpendCap::credits(100);
    let in_credits = budgeted(
        1,
        Some(&Cost::credits(Decimal::from(40), false)),
        Decimal::from(70),
        &credits,
    )
    .unwrap();
    assert_eq!(
        in_credits.message,
        "This job adds 40 credits, bringing today to 110 credits against your 100 credits cap."
    );
}

#[test]
fn review_price_asks_for_the_price_from_higgsfield() {
    let mut priced = job(JobState::Paused(PauseReason::PriceAboveCap));
    priced.server_price = Some(usd("9.5"));
    let model = JobRowModel::new(&priced, now(), None);
    let confirmation = price_confirmation(&priced, &model);
    assert_eq!(confirmation.title, "Start this job for $9.50?");
    assert_eq!(confirmation.confirm_title, "Start for $9.50");
    assert_eq!(
        confirmation.message,
        "This is the price from Higgsfield for the uploaded files. It takes today's spending over your cap."
    );
    let held = job(JobState::Paused(PauseReason::AgentLimit));
    let model = JobRowModel::new(&held, now(), None);
    assert_eq!(
        price_confirmation(&held, &model).message,
        "An agent queued this job: Stopped by the agent spending limits · confirm to send it"
    );
}

#[test]
fn refusals_read_as_the_engine_words_them() {
    let message = user_message(&EngineError::UnknownJob(Uuid::new_v4()), None).unwrap();
    assert_eq!(message.title, "Something went wrong");
    assert_eq!(message.message, "This job is no longer in the queue.");
    assert!(
        user_message(&EngineError::Offline, None)
            .unwrap()
            .message
            .contains("This computer is offline")
    );
    assert_eq!(user_message(&EngineError::Cancelled, None), None);
}

/// `JobPreview`: the source, up to three references with a count of the rest, and a result tile per state.
#[test]
fn the_preview_shows_what_goes_in_and_what_comes_out() {
    let mut rendering = job(JobState::Processing);
    rendering.spec.references = (0..5)
        .map(|index| jutsu_core::LocalImage::new(crate::testing::file_url(format!("/refs/{index}.png"))))
        .collect();
    rendering.spec.references[1] =
        jutsu_core::LocalImage::new(url::Url::parse("https://cdn.example.com/a.png").unwrap());
    let shown = preview(&rendering);
    assert!(shown.has_source);
    assert_eq!(
        shown.source.as_deref(),
        Some(crate::testing::native("/clips/dance.mov").as_str())
    );
    assert_eq!(
        shown.references,
        [
            Some(crate::testing::native("/refs/0.png")),
            None,
            Some(crate::testing::native("/refs/2.png"))
        ]
    );
    assert_eq!(shown.more_references, 2);
    assert_eq!(shown.result, ResultTile::Rendering);
    assert!((shown.aspect - 1280.0 / 720.0).abs() < 1e-9);

    let mut done = job(JobState::Done);
    done.output_file = Some(crate::testing::file_url("/out/result.mp4"));
    assert_eq!(
        preview(&done).result,
        ResultTile::Done {
            path: Some("/out/result.mp4".to_owned())
        }
    );
    assert_eq!(
        preview(&job(JobState::Paused(jutsu_core::PauseReason::InsufficientFunds))).result,
        ResultTile::Paused
    );
    assert_eq!(preview(&job(JobState::Rejected)).result, ResultTile::Failed);
    assert_eq!(preview(&job(JobState::Canceled)).result, ResultTile::Canceled);

    // Without a source: no source tile, and the frame asked for sets the shape, kept within reason.
    let mut portrait = job(JobState::Queued);
    portrait.spec.source = None;
    portrait.spec.aspect_ratio = Some(jutsu_core::AspectRatio::Portrait);
    let shown = preview(&portrait);
    assert!(!shown.has_source && shown.source.is_none());
    assert!((shown.aspect - 0.5625).abs() < 1e-9);
}
