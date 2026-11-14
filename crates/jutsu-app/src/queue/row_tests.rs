//! `JobRowModelTests`, `JobRowStageTests`, `TimeEstimateTests` and `QueueJobCommandsTests` of the macOS app.

use jutsu_core::{
    Cost, FailureReason, JobState, PauseReason, ProviderKind, Rejection, RemoteJob, RetryCause, RetryStatus,
    SubmitAttempt, VideoModel,
};
use rust_decimal::Decimal;
use time::Duration;
use url::Url;

use super::*;
use crate::queue::fixture::{JobFixture, TWO_MINUTES, job, now, timed, usd};

fn row(state: JobState) -> JobRowModel {
    JobRowModel::new(&job(state), now(), None)
}

fn message(text: &str) -> JobState {
    JobState::Failed {
        reason: FailureReason::Message(text.to_owned()),
    }
}

fn cancel_job() -> RowAction {
    RowAction::Cancel {
        is_enabled: true,
        help: "Cancel job".to_owned(),
    }
}

#[allow(clippy::unnecessary_wraps, reason = "compared with `JobRowModel::note`")]
fn note(text: &str, role: ColorRole) -> Option<RowNote> {
    Some(RowNote::new(text, role))
}

#[test]
fn status_label_and_color() {
    for (state, label, role) in [
        (JobState::Preparing, "Preparing", ColorRole::Accent),
        (JobState::Uploading, "Uploading", ColorRole::Accent),
        (JobState::WaitingForSlot, "Waiting for slot", ColorRole::Info),
        (JobState::Queued, "Queued", ColorRole::Info),
        (JobState::Processing, "Processing", ColorRole::Accent),
        (JobState::Downloading, "Downloading", ColorRole::Accent),
        (
            JobState::Paused(PauseReason::InsufficientFunds),
            "Paused",
            ColorRole::Warning,
        ),
        (JobState::Done, "Done", ColorRole::Accent),
        (message("Boom"), "Failed", ColorRole::Danger),
        (JobState::Rejected, "Rejected", ColorRole::Warning),
        (JobState::Canceled, "Canceled", ColorRole::Faint),
    ] {
        let row = row(state.clone());
        assert_eq!(row.status_label, label);
        assert_eq!(row.status_role, role);
        assert_eq!(
            row.is_working,
            state.is_active() && !matches!(state, JobState::Paused(_))
        );
        assert_eq!(row.status_hint.is_some(), state == JobState::WaitingForSlot);
    }
}

#[test]
fn a_job_waiting_for_a_slot_offers_run_next_until_its_submit_is_sent() {
    let mut waiting = job(JobState::WaitingForSlot);
    assert_eq!(
        JobRowModel::new(&waiting, now(), None).actions,
        [RowAction::RunNext, cancel_job()]
    );
    waiting.submit_attempt = Some(SubmitAttempt { is_idempotent: true });
    assert_eq!(JobRowModel::new(&waiting, now(), None).actions, [cancel_job()]);
}

#[test]
fn running_states_offer_cancel() {
    for (state, enabled, expected_help) in [
        (JobState::Preparing, true, "Cancel job"),
        (JobState::Uploading, true, "Cancel job"),
        (JobState::Queued, true, "Cancel job"),
        (JobState::Processing, false, "Can't cancel while processing"),
        (JobState::Downloading, true, "Stop download"),
    ] {
        let row = row(state);
        let [RowAction::Cancel { is_enabled, help }] = row.actions.as_slice() else {
            panic!("expected cancel, got {:?}", row.actions);
        };
        assert_eq!(*is_enabled, enabled);
        assert_eq!(help, expected_help);
        assert_eq!(row.note, None);
    }
}

#[test]
fn the_row_file_is_the_finished_video() {
    let output = Url::parse("file:///Movies/out.mp4").unwrap();
    let mut done = job(JobState::Done);
    done.output_file = Some(output.clone());
    let mut failed = job(message("x"));
    failed.output_file = Some(output);
    assert_eq!(
        JobRowModel::new(&done, now(), None).file(),
        Some("/Movies/out.mp4")
    );
    assert_eq!(row(JobState::Done).file(), None);
    assert_eq!(JobRowModel::new(&failed, now(), None).file(), None);
}

#[test]
fn terminal_and_paused_actions() {
    let mut done = job(JobState::Done);
    done.output_file = Some(Url::parse("file:///Movies/out.mp4").unwrap());
    assert_eq!(
        JobRowModel::new(&done, now(), None).actions,
        [
            RowAction::ShowInFolder {
                path: "/Movies/out.mp4".to_owned()
            },
            RowAction::OpenInLibrary
        ]
    );
    assert_eq!(row(JobState::Done).actions, [RowAction::OpenInLibrary]);
    assert_eq!(row(message("x")).actions, [RowAction::Retry]);
    assert_eq!(row(JobState::Canceled).actions, [RowAction::Retry]);
    assert_eq!(row(JobState::Rejected).actions, [RowAction::Edit]);
    for state in [message("Download failed"), JobState::Canceled] {
        let mut paid = job(state);
        paid.remote = Some(RemoteJob::new("r1"));
        paid.charged_at = Some(now());
        assert_eq!(
            JobRowModel::new(&paid, now(), None).actions,
            [RowAction::RetryDownload, RowAction::GenerateAgain]
        );
    }
    let mut timed_out = job(JobState::Failed {
        reason: FailureReason::Deadline,
    });
    timed_out.remote = Some(RemoteJob::new("r1"));
    timed_out.timed_out_at = Some(now());
    assert_eq!(
        JobRowModel::new(&timed_out, now(), None).actions,
        [RowAction::CheckAgain]
    );
    assert_eq!(
        row(JobState::Paused(PauseReason::InsufficientFunds)).actions,
        [RowAction::Resume, cancel_job()]
    );
}

#[test]
fn not_charged_states_show_zero() {
    for state in [
        message("Input video could not be downloaded"),
        JobState::Rejected,
        JobState::Canceled,
    ] {
        let usd = row(state.clone());
        assert!(!usd.is_charged);
        assert_eq!(usd.cost, "$0.00");
        assert!(usd.meta.contains(" · $0.00 · "));
        assert!(usd.note.unwrap().text.to_lowercase().contains("not charged"));
        let credits = JobFixture {
            kind: ProviderKind::Account,
            cost: Cost::credits(Decimal::from(96), false),
            ..JobFixture::default()
        }
        .job(state);
        assert_eq!(JobRowModel::new(&credits, now(), None).cost, "0 cr");
    }
}

#[test]
fn jobs_the_api_finished_stay_charged_when_the_download_stops() {
    let mut canceled = job(JobState::Canceled);
    canceled.charged_at = Some(now());
    let canceled_row = JobRowModel::new(&canceled, now(), None);
    assert!(canceled_row.is_charged);
    assert_eq!(canceled_row.cost, "$8.17");
    assert_eq!(
        canceled_row.note,
        note("Download canceled · charged", ColorRole::Faint)
    );
    let mut failed = job(message("Download failed"));
    failed.charged_at = Some(now());
    assert_eq!(
        JobRowModel::new(&failed, now(), None).note,
        note("Download failed · charged", ColorRole::Danger)
    );
}

#[test]
fn failed_and_rejected_notes() {
    assert_eq!(
        row(message("Input video could not be downloaded")).note,
        note(
            "Input video could not be downloaded · not charged",
            ColorRole::Danger
        )
    );
    assert_eq!(row(message(" ")).note.unwrap().text, "Failed · not charged");
    assert_eq!(
        row(JobState::Failed {
            reason: FailureReason::Deadline
        })
        .note
        .unwrap()
        .text,
        "Higgsfield didn't finish this job in time. Check the result in your generations on Higgsfield. · not charged"
    );
    assert_eq!(
        row(JobState::Failed {
            reason: FailureReason::ContentModeration
        })
        .note
        .unwrap()
        .text,
        "Rejected by content moderation. · not charged"
    );
    assert_eq!(
        row(JobState::Rejected).note,
        note("Rejected by content moderation · not charged", ColorRole::Warning)
    );
    let mut protected = job(JobState::Rejected);
    protected.rejection = Some(Rejection::ProtectedContent);
    assert_eq!(
        JobRowModel::new(&protected, now(), None).note.unwrap().text,
        "Rejected: a recognizable person or protected content · not charged"
    );
}

#[test]
fn the_paused_reason_depends_on_the_connection() {
    assert_eq!(
        row(JobState::Paused(PauseReason::InsufficientFunds))
            .note
            .unwrap()
            .text,
        "Not enough API balance · top up, then resume"
    );
    let account = JobFixture {
        kind: ProviderKind::Account,
        ..JobFixture::default()
    }
    .job(JobState::Paused(PauseReason::InsufficientFunds));
    assert_eq!(
        JobRowModel::new(&account, now(), None).note.unwrap().text,
        "Out of plan credits · top up, then resume"
    );
    assert_eq!(
        row(JobState::Paused(PauseReason::ConnectionNeedsAttention)).note,
        note(
            "Connection needs attention · sign in again, then resume",
            ColorRole::Warning
        )
    );
    let by_user = row(JobState::Paused(PauseReason::ByUser));
    assert_eq!(
        by_user.note,
        note(
            "Paused by you · nothing is submitted until you resume",
            ColorRole::Faint
        )
    );
    assert_eq!(by_user.status_label, "Paused");
}

#[test]
fn a_job_paused_for_disk_space_resumes() {
    let row = row(JobState::Paused(PauseReason::NotEnoughDiskSpace));
    assert_eq!(
        row.note,
        note(
            "Not enough disk space to save the result · free up space, then resume",
            ColorRole::Warning
        )
    );
    assert_eq!(
        row.actions,
        [
            RowAction::Resume,
            RowAction::Cancel {
                is_enabled: true,
                help: "Stop download".to_owned()
            }
        ]
    );
}

#[test]
fn a_retrying_job_says_when_it_tries_again_and_why() {
    let mut retrying = job(JobState::Uploading);
    retrying.retrying = Some(RetryStatus {
        failures: 3,
        since: now() - Duration::seconds(60),
        next_attempt_at: now() + Duration::milliseconds(29_400),
        cause: RetryCause::Server { status: 502 },
    });
    let row = JobRowModel::new(&retrying, now(), None);
    assert_eq!(
        row.note,
        note(
            "Retrying in 30 s · attempt 4 · last error: Higgsfield server error (HTTP 502)",
            ColorRole::Warning
        )
    );
    assert_eq!(row.status_label, "Uploading");
    assert_eq!(row.actions, [cancel_job()]);
    let due = JobRowModel::new(&retrying, now() + Duration::seconds(40), None);
    assert_eq!(
        due.note.unwrap().text,
        "Retrying · attempt 4 · last error: Higgsfield server error (HTTP 502)"
    );
    retrying.retrying.as_mut().unwrap().cause = RetryCause::Network;
    assert_eq!(
        JobRowModel::new(&retrying, now(), None).note.unwrap().text,
        "Retrying in 30 s · attempt 4 · last error: no connection to Higgsfield"
    );
    assert_eq!(super::tests::row(JobState::Uploading).note, None);
}

#[test]
fn a_job_paused_after_failing_says_why_and_resumes() {
    let mut kept_failing = job(JobState::Paused(PauseReason::KeptFailing));
    kept_failing.retrying = Some(RetryStatus {
        failures: 19,
        since: now() - Duration::seconds(900),
        next_attempt_at: now(),
        cause: RetryCause::ModelUnavailable,
    });
    let row = JobRowModel::new(&kept_failing, now(), None);
    assert_eq!(
        row.note,
        note(
            "Kept failing before it was sent (the model is unavailable on Higgsfield) · not charged, resume to try again",
            ColorRole::Warning
        )
    );
    assert_eq!(row.actions, [RowAction::Resume, cancel_job()]);
    assert_eq!(
        super::tests::row(JobState::Paused(PauseReason::KeptFailing))
            .note
            .unwrap()
            .text,
        "Kept failing before it was sent · not charged, resume to try again"
    );
}

#[test]
fn a_job_paused_for_its_server_price_asks_before_resuming() {
    let mut priced = job(JobState::Paused(PauseReason::PriceAboveCap));
    priced.server_price = Some(usd("9.5"));
    let row = JobRowModel::new(&priced, now(), None);
    assert_eq!(row.actions, [RowAction::ConfirmPrice, cancel_job()]);
    assert_eq!(
        row.note.unwrap().text,
        "Price from Higgsfield goes over today's spending cap · confirm to resume"
    );
    assert_eq!(row.cost, "$9.50");
    assert_eq!(row.price_source, "Price from Higgsfield");
    assert_eq!(super::tests::row(JobState::Queued).price_source, "Estimate");
}

#[test]
fn meta_line() {
    let uploading = JobFixture {
        progress: 0.424,
        ..JobFixture::default()
    }
    .job(JobState::Uploading);
    let row = JobRowModel::new(&uploading, now(), None);
    assert_eq!(row.title, "dance · Motion transfer");
    assert_eq!(
        row.meta,
        "Motion transfer · 720p · 00:00.0–00:12.0 · $8.17 · 3 min ago"
    );
    assert_eq!(row.progress, Some(0.424));
    assert_eq!(row.progress_text.as_deref(), Some("42%"));
}

#[test]
fn the_meta_line_names_a_model_other_than_genjutsu() {
    let mut seedance = JobFixture {
        kind: ProviderKind::Account,
        cost: Cost::credits(Decimal::from(70), false),
        ..JobFixture::default()
    }
    .job(JobState::Done);
    seedance.spec.model = VideoModel::SEEDANCE25;
    assert!(
        JobRowModel::new(&seedance, now(), None)
            .meta
            .starts_with("Motion transfer · Seedance 2.5 · 720p · ")
    );
}

#[test]
fn meta_line_with_credits_and_finished_time() {
    let done = JobFixture {
        mode: jutsu_core::Mode::Restyle,
        kind: ProviderKind::Account,
        cost: Cost::credits(Decimal::from(96), false),
        created_minutes_ago: 300.0,
        finished_minutes_ago: Some(120.0),
        ..JobFixture::default()
    }
    .job(JobState::Done);
    let row = JobRowModel::new(&done, now(), None);
    assert_eq!(row.meta, "Restyle · 720p · 00:00.0–00:12.0 · 96 cr · 2 h ago");
    assert_eq!(row.progress, None);
    assert_eq!(row.progress_text, None);
}

#[test]
fn progress_is_clamped_and_only_for_reporting_states() {
    let with = |state, progress| {
        JobRowModel::new(
            &JobFixture {
                progress,
                ..JobFixture::default()
            }
            .job(state),
            now(),
            None,
        )
    };
    assert_eq!(
        with(JobState::Downloading, 1.4).progress_text.as_deref(),
        Some("100%")
    );
    assert_eq!(with(JobState::Queued, 0.5).progress, None);
    assert_eq!(with(JobState::Processing, 0.0).progress_text, None);
}

#[test]
fn the_stage_follows_the_state() {
    for (state, stage) in [
        (JobState::Preparing, Some(Stage::Prepare)),
        (JobState::Uploading, Some(Stage::Upload)),
        (JobState::WaitingForSlot, Some(Stage::Upload)),
        (JobState::Queued, Some(Stage::Generate)),
        (JobState::Processing, Some(Stage::Generate)),
        (JobState::Downloading, Some(Stage::Save)),
        (JobState::Paused(PauseReason::InsufficientFunds), None),
        (JobState::Done, None),
        (message(""), None),
        (JobState::Canceled, None),
    ] {
        assert_eq!(row(state).stage, stage);
    }
}

#[test]
fn elapsed_only_while_active_and_a_retry_restarts_it() {
    assert_eq!(row(JobState::Processing).elapsed.as_deref(), Some("3:00"));
    assert_eq!(row(JobState::Done).elapsed, None);
    let mut retried = JobFixture {
        created_minutes_ago: 13.0,
        ..JobFixture::default()
    }
    .job(JobState::Processing);
    retried.retried_at = Some(now() - Duration::seconds(90));
    let row = JobRowModel::new(&retried, now(), None);
    assert_eq!(row.elapsed.as_deref(), Some("1:30"));
    assert!(row.meta.ends_with("2 min ago"));
}

#[test]
fn the_prompt_is_one_line_or_none() {
    let mut processing = job(JobState::Processing);
    processing.spec.prompt = "  Keep the car\nswap the sky  ".to_owned();
    assert_eq!(
        JobRowModel::new(&processing, now(), None).prompt.as_deref(),
        Some("Keep the car swap the sky")
    );
    processing.spec.prompt = " \n ".to_owned();
    assert_eq!(JobRowModel::new(&processing, now(), None).prompt, None);
}

// MARK: TimeEstimateTests

#[test]
fn a_generating_job_fills_the_bar_against_the_typical_time() {
    let row = JobRowModel::new(&timed(JobState::Processing, 1.0, None), now(), Some(&TWO_MINUTES));
    let estimate = row.time_estimate.unwrap();
    assert_eq!(estimate.label, "usually ~2 min");
    assert!((estimate.fraction - 0.5).abs() < 1e-9);
    assert!(!estimate.is_overdue);
    assert_eq!(estimate.help, "Estimate from 4 past jobs");
    assert_eq!(row.progress, None);
}

#[test]
fn past_the_estimate_the_bar_stays_full() {
    let estimate = JobRowModel::new(&timed(JobState::Processing, 5.0, None), now(), Some(&TWO_MINUTES))
        .time_estimate
        .unwrap();
    assert_eq!(estimate.label, "taking longer than usual");
    assert!((estimate.fraction - 1.0).abs() < 1e-9);
    assert!(estimate.is_overdue);
}

#[test]
fn queued_at_the_api_counts_as_generating() {
    let fraction = JobRowModel::new(&timed(JobState::Queued, 0.5, None), now(), Some(&TWO_MINUTES))
        .time_estimate
        .unwrap()
        .fraction;
    assert!((fraction - 0.25).abs() < 1e-9);
}

#[test]
fn only_the_generate_step_has_an_estimate() {
    for state in [
        JobState::Uploading,
        JobState::WaitingForSlot,
        JobState::Downloading,
        JobState::Paused(PauseReason::ByUser),
        JobState::Done,
    ] {
        let row = JobRowModel::new(&timed(state, 4.0, None), now(), Some(&TWO_MINUTES));
        assert_eq!(row.time_estimate, None);
    }
}

#[test]
fn the_generate_step_times_the_generation_alone() {
    let generating = |state, minutes| JobRowModel::new(&timed(state, minutes, None), now(), None).generating;
    assert_eq!(generating(JobState::Processing, 2.0).as_deref(), Some("2:00"));
    assert_eq!(generating(JobState::Queued, 0.5).as_deref(), Some("0:30"));
    assert_eq!(generating(JobState::Downloading, 4.0), None);
    let mut unsubmitted = timed(JobState::Processing, 4.0, None);
    unsubmitted.submitted_at = None;
    assert_eq!(JobRowModel::new(&unsubmitted, now(), None).generating, None);
}

#[test]
fn without_history_or_submit_time_only_the_timer_shows() {
    let processing = timed(JobState::Processing, 4.0, None);
    let row = JobRowModel::new(&processing, now(), None);
    assert_eq!(row.time_estimate, None);
    assert_eq!(row.elapsed.as_deref(), Some("5:00"));
    let mut unsubmitted = processing;
    unsubmitted.submitted_at = None;
    assert_eq!(
        JobRowModel::new(&unsubmitted, now(), Some(&TWO_MINUTES)).time_estimate,
        None
    );
    let zero = GenerationEstimate {
        queue_wait: 0.0,
        processing: 0.0,
        sample_count: 3,
    };
    assert_eq!(time_estimate(10.0, &zero), None);
}

// MARK: QueueJobCommandsTests

fn commands(job: &jutsu_core::Job) -> QueueJobCommands {
    QueueJobCommands::new(&JobRowModel::new(job, now(), None))
}

#[test]
fn cancel_follows_the_rows_cancel_button() {
    assert!(commands(&job(JobState::Queued)).can_cancel);
    assert!(commands(&job(JobState::Paused(PauseReason::InsufficientFunds))).can_cancel);
    assert!(!commands(&job(JobState::Processing)).can_cancel);
    assert!(!commands(&job(JobState::Done)).can_cancel);
    assert!(!commands(&job(message("x"))).can_cancel);
}

#[test]
fn retry_runs_what_the_row_offers() {
    let failed = commands(&job(message("x")));
    assert_eq!(failed.retry, Some(JobCommand::Retry));
    assert_eq!(failed.retry_title, "Retry Job");

    let mut paid = job(message("Download failed"));
    paid.remote = Some(RemoteJob::new("r1"));
    paid.charged_at = Some(now());
    let download = commands(&paid);
    assert_eq!(download.retry, Some(JobCommand::Retry));
    assert_eq!(download.retry_title, "Retry Download");

    let mut timed_out = job(JobState::Failed {
        reason: FailureReason::Deadline,
    });
    timed_out.remote = Some(RemoteJob::new("r1"));
    timed_out.timed_out_at = Some(now());
    let check_again = commands(&timed_out);
    assert_eq!(check_again.retry, Some(JobCommand::CheckAgain));
    assert_eq!(check_again.retry_title, "Check Again");
}

#[test]
fn nothing_to_retry() {
    for state in [
        JobState::Processing,
        JobState::Done,
        JobState::Rejected,
        JobState::Paused(PauseReason::InsufficientFunds),
    ] {
        assert_eq!(commands(&job(state)).retry, None);
    }
}

#[test]
fn agent_jobs_name_their_origin() {
    let mut queued = job(JobState::Queued);
    queued.agent = Some(jutsu_core::AgentOrigin::new(
        Some("claude-code".to_owned()),
        "req-1",
        AgentMoney::new(200, AgentMoneyUnit::UsdCents),
    ));
    let row = JobRowModel::new(&queued, now(), None);
    assert_eq!(row.origin.as_deref(), Some("Agent · claude-code"));
    assert_eq!(row.origin_detail.as_deref(), Some("Allowed up to $2.00"));
}
