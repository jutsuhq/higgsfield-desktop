//! `JobNotificationTests`.

use std::collections::HashMap;

use jutsu_core::{FailureReason, Job, JobState, PauseReason};
use time::Duration;
use url::Url;
use uuid::Uuid;

use super::*;
use crate::preferences::NotificationPreferences;
use crate::queue::fixture::{self, JobFixture, now};

fn since() -> OffsetDateTime {
    now() - Duration::hours(1)
}

fn all() -> NotificationPreferences {
    NotificationPreferences::default()
}

fn off() -> NotificationPreferences {
    NotificationPreferences {
        on_done: false,
        on_problems: false,
        plays_sound: true,
    }
}

fn changes_with(
    previous: &[Job],
    current: &[Job],
    preferences: NotificationPreferences,
) -> Vec<JobNotification> {
    let states = previous
        .iter()
        .map(|job| (job.id, job.state.clone()))
        .collect::<HashMap<_, _>>();
    super::changes(&states, current, since(), preferences)
}

fn changes(previous: &[Job], current: &[Job]) -> Vec<JobNotification> {
    changes_with(previous, current, all())
}

fn moved(job: &Job, state: JobState) -> Job {
    let mut copy = job.clone();
    copy.state = state;
    copy.progress = 0.0;
    if copy.state.is_terminal() {
        copy.finished_at = Some(now());
    }
    copy
}

fn failed(message: &str) -> JobState {
    JobState::Failed {
        reason: FailureReason::Message(message.to_owned()),
    }
}

fn processing() -> Job {
    JobFixture {
        progress: 0.5,
        ..JobFixture::default()
    }
    .job(JobState::Processing)
}

#[test]
fn each_notifying_transition() {
    let cases = [
        (
            JobState::Done,
            NotificationKind::Done,
            "Video ready",
            Destination::Library,
        ),
        (
            failed("Bad input"),
            NotificationKind::Failed,
            "Job failed",
            Destination::Queue,
        ),
        (
            JobState::Rejected,
            NotificationKind::Rejected,
            "Job rejected",
            Destination::Queue,
        ),
        (
            JobState::Paused(PauseReason::InsufficientFunds),
            NotificationKind::Paused,
            "Job paused",
            Destination::Queue,
        ),
        (
            JobState::Paused(PauseReason::ConnectionNeedsAttention),
            NotificationKind::Paused,
            "Job paused",
            Destination::Queue,
        ),
    ];
    for (state, kind, title, destination) in cases {
        let running = processing();
        let posted = changes(std::slice::from_ref(&running), &[moved(&running, state)]);
        assert_eq!(posted.len(), 1);
        let notification = &posted[0];
        assert_eq!(notification.job_id, running.id);
        assert_eq!(notification.kind, kind);
        assert_eq!(notification.title, title);
        assert_eq!(notification.destination(), destination);
        assert_eq!(notification.subtitle, "dance · Motion transfer");
    }
}

#[test]
fn body_carries_the_reason() {
    let running = processing();
    let failed = changes(
        std::slice::from_ref(&running),
        &[moved(&running, failed("Bad input"))],
    );
    assert_eq!(failed[0].body, "Bad input · not charged");
    assert_eq!(
        failed[0].display_body(),
        "dance · Motion transfer\nBad input · not charged"
    );
    let done = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]);
    assert_eq!(done[0].body, "");
    assert_eq!(done[0].display_body(), "dance · Motion transfer");
}

#[test]
fn a_done_job_says_how_long_it_generated() {
    let running = fixture::timed(JobState::Processing, 5.0, Some(100.0));
    let done = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]);
    // Submitted, 10 s in the API's queue, 100 s generating.
    assert_eq!(done[0].body, "Generated in 1 min 50 s");
}

#[test]
fn only_a_video_ready_carries_the_result_file() {
    let mut running = processing();
    running.output_file = Some(Url::parse("file:///tmp/dance.mp4").unwrap());
    let done = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]);
    assert!(
        done[0]
            .result_file
            .as_ref()
            .is_some_and(|file| file.ends_with("dance.mp4"))
    );
    assert_eq!(done[0].category, Some(NotificationCategory::Result));
    let failed = changes(
        std::slice::from_ref(&running),
        &[moved(&running, failed("Bad input"))],
    );
    assert_eq!(failed[0].result_file, None);
}

#[test]
fn the_buttons_follow_the_result_and_the_retry() {
    let mut draft = processing();
    draft.output_file = Some(Url::parse("file:///tmp/dance.mp4").unwrap());
    draft.spec.is_draft = true;
    let done = changes(std::slice::from_ref(&draft), &[moved(&draft, JobState::Done)]);
    assert_eq!(done[0].category, Some(NotificationCategory::DraftResult));
    assert_eq!(
        NotificationCategory::DraftResult
            .actions()
            .iter()
            .map(|action| action.title())
            .collect::<Vec<_>>(),
        ["Render Final…", "Show in File Manager"]
    );
    let running = processing();
    let without_file = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]);
    assert_eq!(without_file[0].category, None);
    let rejected = changes(
        std::slice::from_ref(&running),
        &[moved(&running, JobState::Rejected)],
    );
    assert_eq!(rejected[0].category, None);
}

#[test]
fn a_batch_shares_one_thread() {
    let mut running = processing();
    assert_eq!(
        changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)])[0].thread_id,
        None
    );
    let batch = Uuid::new_v4();
    running.batch_id = Some(batch);
    assert_eq!(
        changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)])[0].thread_id,
        Some(batch.to_string().to_uppercase())
    );
}

#[test]
fn progress_and_in_flight_changes_post_nothing() {
    let job = JobFixture {
        progress: 0.1,
        ..JobFixture::default()
    }
    .job(JobState::Uploading);
    let mut further = job.clone();
    further.progress = 0.9;
    assert_eq!(changes(std::slice::from_ref(&job), &[further]), []);
    assert_eq!(
        changes(std::slice::from_ref(&job), &[moved(&job, JobState::Processing)]),
        []
    );
    assert_eq!(
        changes(std::slice::from_ref(&job), &[moved(&job, JobState::Canceled)]),
        []
    );
}

#[test]
fn same_kind_twice_is_no_duplicate() {
    let done = moved(&processing(), JobState::Done);
    assert_eq!(
        changes(std::slice::from_ref(&done), std::slice::from_ref(&done)),
        []
    );
    let paused = fixture::job(JobState::Paused(PauseReason::InsufficientFunds));
    assert_eq!(
        changes(
            std::slice::from_ref(&paused),
            &[moved(
                &paused,
                JobState::Paused(PauseReason::ConnectionNeedsAttention)
            )]
        ),
        []
    );
}

#[test]
fn pauses_by_the_person_post_nothing_but_their_system_pause_does() {
    let waiting = fixture::job(JobState::WaitingForSlot);
    assert_eq!(
        changes(
            std::slice::from_ref(&waiting),
            &[moved(&waiting, JobState::Paused(PauseReason::ByUser))]
        ),
        []
    );
    let held = fixture::job(JobState::Paused(PauseReason::ByUser));
    let posted = changes(
        std::slice::from_ref(&held),
        &[moved(
            &held,
            JobState::Paused(PauseReason::ConnectionNeedsAttention),
        )],
    );
    assert_eq!(
        posted
            .iter()
            .map(|notification| notification.kind)
            .collect::<Vec<_>>(),
        [NotificationKind::Paused]
    );
}

#[test]
fn done_and_problems_are_separate_switches() {
    let running = processing();
    let only_done = NotificationPreferences {
        on_problems: false,
        ..all()
    };
    let only_problems = NotificationPreferences {
        on_done: false,
        ..all()
    };
    let kinds = |posted: Vec<JobNotification>| {
        posted
            .iter()
            .map(|notification| notification.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(changes_with(
            std::slice::from_ref(&running),
            &[moved(&running, JobState::Done)],
            only_done
        )),
        [NotificationKind::Done]
    );
    assert_eq!(
        changes_with(
            std::slice::from_ref(&running),
            &[moved(&running, JobState::Done)],
            only_problems
        ),
        []
    );
    for state in [
        failed("x"),
        JobState::Rejected,
        JobState::Paused(PauseReason::InsufficientFunds),
    ] {
        assert_eq!(
            changes_with(
                std::slice::from_ref(&running),
                &[moved(&running, state.clone())],
                only_done
            ),
            []
        );
        assert_eq!(
            changes_with(
                std::slice::from_ref(&running),
                &[moved(&running, state)],
                only_problems
            )
            .len(),
            1
        );
    }
}

#[test]
fn sound_follows_the_setting() {
    let running = processing();
    let quiet = NotificationPreferences {
        plays_sound: false,
        ..all()
    };
    assert!(changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)])[0].plays_sound);
    assert!(
        !changes_with(
            std::slice::from_ref(&running),
            &[moved(&running, JobState::Done)],
            quiet
        )[0]
        .plays_sound
    );
}

#[test]
fn unseen_jobs_notify_only_if_they_finished_while_observing() {
    let stored = fixture::finished(JobState::Done, 300.0, 120.0);
    let fresh = fixture::finished(failed("x"), 3.0, 1.0);
    let restored_pause = fixture::job(JobState::Paused(PauseReason::InsufficientFunds));
    let posted = changes(&[], &[stored, fresh.clone(), restored_pause]);
    assert_eq!(
        posted
            .iter()
            .map(|notification| notification.job_id)
            .collect::<Vec<_>>(),
        [fresh.id]
    );
}

#[test]
fn retried_job_notifies_again() {
    let running = processing();
    let mut tracker = JobTransitionTracker::new(since());
    assert_eq!(tracker.update(std::slice::from_ref(&running), all()), []);
    assert_eq!(tracker.update(&[moved(&running, failed("x"))], all()).len(), 1);
    assert_eq!(tracker.update(&[moved(&running, failed("x"))], all()), []);
    assert_eq!(tracker.update(&[moved(&running, JobState::Queued)], all()), []);
    assert_eq!(
        tracker
            .update(&[moved(&running, JobState::Done)], all())
            .iter()
            .map(|notification| notification.kind)
            .collect::<Vec<_>>(),
        [NotificationKind::Done]
    );
}

#[test]
fn tracker_remembers_while_disabled() {
    let running = processing();
    let mut tracker = JobTransitionTracker::new(since());
    let _ = tracker.update(std::slice::from_ref(&running), all());
    assert_eq!(tracker.update(&[moved(&running, JobState::Done)], off()), []);
    assert_eq!(tracker.update(&[moved(&running, JobState::Done)], all()), []);
}

#[test]
fn user_info_round_trips() {
    let mut running = processing();
    running.output_file = Some(Url::parse("file:///tmp/dance.mp4").unwrap());
    let notification = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]).remove(0);
    let info = notification.user_info();
    assert_eq!(target(&info), Some((Destination::Library, running.id)));
    let bad = HashMap::from([
        ("jobID".to_owned(), "nope".to_owned()),
        ("destination".to_owned(), "queue".to_owned()),
    ]);
    assert_eq!(target(&bad), None);
    assert_eq!(target(&HashMap::new()), None);

    let click = NotificationResponse::new(&info, DEFAULT_ACTION_ID).unwrap();
    assert_eq!(click.action, None);
    assert_eq!(click.destination, Destination::Library);
    assert!(click.result_file.is_some_and(|file| file.ends_with("dance.mp4")));
    let button = NotificationResponse::new(&info, "showInFinder").unwrap();
    assert_eq!(button.action, Some(NotificationAction::ShowInFolder));
    assert_eq!(NotificationResponse::new(&info, "dismiss"), None);
}

#[test]
fn a_click_opens_the_screen_and_a_button_does_what_it_does_in_the_app() {
    let id = Uuid::new_v4();
    let response = |destination, action, result_file: Option<&str>| NotificationResponse {
        destination,
        job_id: id,
        action,
        result_file: result_file.map(std::path::PathBuf::from),
    };
    let library = TrayOpen::LibraryJob { id: id.to_string() };
    let queue = TrayOpen::QueueJob { id: id.to_string() };
    assert_eq!(
        response(Destination::Library, None, None).effect(),
        NotificationEffect::Open(library.clone())
    );
    assert_eq!(
        response(Destination::Queue, None, None).effect(),
        NotificationEffect::Open(queue.clone())
    );
    assert_eq!(
        response(Destination::Library, Some(NotificationAction::RenderFinal), None).effect(),
        NotificationEffect::Open(library)
    );
    assert_eq!(
        response(
            Destination::Library,
            Some(NotificationAction::ShowInFolder),
            Some("/tmp/a.mp4")
        )
        .effect(),
        NotificationEffect::Reveal("/tmp/a.mp4".into())
    );
    assert_eq!(
        response(Destination::Library, Some(NotificationAction::ShowInFolder), None).effect(),
        NotificationEffect::Open(queue)
    );
    assert_eq!(
        response(Destination::Queue, Some(NotificationAction::Retry), None).effect(),
        NotificationEffect::Retry(id)
    );
}

/// Every notification of one job carries one identifier, so a later state replaces the earlier banner.
#[test]
fn a_jobs_notifications_share_one_identifier() {
    let running = processing();
    let failed_job = moved(&running, failed("x"));
    let done = changes(std::slice::from_ref(&running), &[moved(&running, JobState::Done)]);
    let failed = changes(std::slice::from_ref(&running), &[failed_job]);
    assert_eq!(done[0].notification_id(), failed[0].notification_id());

    let mut other = processing();
    other.id = Uuid::from_u128(0x0000_0001_0000_0002_0000_0004_0000_0008);
    let notification = &changes(std::slice::from_ref(&other), &[moved(&other, JobState::Done)])[0];
    assert_eq!(notification.notification_id(), 1 ^ 2 ^ 4 ^ 8);
}
