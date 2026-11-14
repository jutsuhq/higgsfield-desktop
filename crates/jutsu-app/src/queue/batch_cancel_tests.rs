//! `BatchCancelTests`, ported.

use jutsu_core::{Job, JobState, PauseReason, RemoteJob, SubmitAttempt};
use uuid::Uuid;

use super::*;
use crate::queue::fixture::JobFixture;

fn batch(jobs: Vec<Job>) -> JobBatch {
    JobBatch::new(Uuid::new_v4(), jobs)
}

/// Each job a minute older than the next, so the batch's order never rests on equal timestamps.
fn job(state: JobState, minutes_ago: f64, sent: bool) -> Job {
    let mut job = JobFixture {
        created_minutes_ago: minutes_ago,
        ..JobFixture::default()
    }
    .job(state);
    if sent {
        job.remote = Some(RemoteJob::new(format!("req-{minutes_ago}")));
    }
    job
}

fn ids(jobs: &[Job]) -> Vec<Uuid> {
    jobs.iter().map(|job| job.id).collect()
}

#[test]
fn cancels_only_what_canceling_still_saves() {
    let waiting = job(JobState::WaitingForSlot, 9.0, false);
    let queued = job(JobState::Queued, 8.0, true);
    let paused = job(JobState::Paused(PauseReason::ByUser), 7.0, false);
    let processing = job(JobState::Processing, 6.0, true);
    let downloading = job(JobState::Downloading, 5.0, true);
    let no_disk = job(JobState::Paused(PauseReason::NotEnoughDiskSpace), 4.0, true);
    let done = job(JobState::Done, 3.0, true);
    let cancel = BatchCancel::new(&batch(vec![
        done,
        no_disk,
        downloading,
        processing,
        paused.clone(),
        queued.clone(),
        waiting.clone(),
    ]));
    assert_eq!(ids(&cancel.jobs), [waiting.id, queued.id, paused.id]);
    assert_eq!(cancel.submitted, 1);
    assert!(!cancel.is_empty());
}

#[test]
fn nothing_left_to_cancel() {
    let cancel = BatchCancel::new(&batch(vec![
        job(JobState::Processing, 2.0, true),
        job(JobState::Done, 1.0, false),
    ]));
    assert!(cancel.is_empty());
    assert_eq!(cancel.confirmation(), None);
}

#[test]
fn asks_only_once_a_job_was_sent() {
    let local = BatchCancel::new(&batch(vec![
        job(JobState::Preparing, 2.0, false),
        job(JobState::WaitingForSlot, 1.0, false),
    ]));
    assert_eq!(local.confirmation(), None);

    let mut submitting = job(JobState::WaitingForSlot, 1.0, false);
    submitting.submit_attempt = Some(SubmitAttempt { is_idempotent: true });
    let sent = BatchCancel::new(&batch(vec![
        job(JobState::Queued, 3.0, true),
        job(JobState::Uploading, 2.0, false),
        submitting,
    ]));
    assert_eq!(sent.submitted, 2);
    let confirmation = sent.confirmation().unwrap();
    assert_eq!(confirmation.title, "Cancel 3 jobs?");
    assert_eq!(confirmation.confirm_title, "Cancel 3 Jobs");
    assert_eq!(
        confirmation.message,
        "2 of them are already with Higgsfield, which is asked to drop them. A job it already started can't be \
         canceled and is charged."
    );

    let one = BatchCancel::new(&batch(vec![job(JobState::Queued, 1.0, true)]));
    let confirmation = one.confirmation().unwrap();
    assert_eq!(confirmation.title, "Cancel 1 job?");
    assert!(
        confirmation
            .message
            .starts_with("1 of them is already with Higgsfield")
    );
}

#[test]
fn outcome_names_how_many_were_refused() {
    let refused = UserMessage::new("Can't cancel", "It is already processing.", None);
    assert_eq!(BatchCancel::outcome(&[], 3), None);
    assert_eq!(
        BatchCancel::outcome(std::slice::from_ref(&refused), 1),
        Some(refused.clone())
    );
    let some = BatchCancel::outcome(&[refused.clone(), refused], 4).unwrap();
    assert_eq!(some.title, "2 of 4 jobs weren't canceled");
    assert_eq!(some.message, "Can't cancel. It is already processing.");
}
