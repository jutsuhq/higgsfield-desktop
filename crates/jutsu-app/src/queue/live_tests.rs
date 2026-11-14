//! `LiveJobsTests`, `LiveJobsGenerationTimesTests` and `QueueProblemsTests` of the macOS app.

use jutsu_core::{FailureReason, GenerationTimes, Job, JobState, PauseReason, SubmitAttempt};
use time::UtcOffset;

use super::*;
use crate::queue::fixture::{JobFixture, finished, history, job, minutes_ago, now, timed};

const UTC: UtcOffset = UtcOffset::UTC;

fn failed(text: &str) -> JobState {
    JobState::Failed {
        reason: FailureReason::Message(text.to_owned()),
    }
}

fn created(state: JobState, minutes: f64) -> Job {
    JobFixture {
        created_minutes_ago: minutes,
        ..JobFixture::default()
    }
    .job(state)
}

fn ids(jobs: &[&Job]) -> Vec<Uuid> {
    jobs.iter().map(|job| job.id).collect()
}

fn entry_ids(entries: &[QueueEntry]) -> Vec<Uuid> {
    entries.iter().map(QueueEntry::id).collect()
}

/// Applies at the fixture's `now`, in UTC.
fn apply(live: &mut LiveJobs, job: Job) {
    live.apply(job, now(), UTC);
}

#[test]
fn active_is_oldest_first_and_paused_counts() {
    let newer = created(JobState::Processing, 1.0);
    let older = created(JobState::Paused(PauseReason::InsufficientFunds), 5.0);
    let done = created(JobState::Done, 10.0);
    let live = LiveJobs::new(&[newer.clone(), done, older.clone()]);
    assert_eq!(ids(&live.active()), [older.id, newer.id]);
    assert_eq!(live.active_count(), 2);
}

#[test]
fn apply_replaces_by_id() {
    let mut uploading = JobFixture {
        progress: 0.2,
        ..JobFixture::default()
    }
    .job(JobState::Uploading);
    let mut live = LiveJobs::new(std::slice::from_ref(&uploading));
    uploading.progress = 0.6;
    apply(&mut live, uploading.clone());
    assert!((live.job(uploading.id).unwrap().progress - 0.6).abs() < 1e-9);
    uploading.state = JobState::Done;
    apply(&mut live, uploading.clone());
    assert_eq!(live.active().len(), 0);
    assert_eq!(ids(&live.recent_finished(5)), [uploading.id]);
}

#[test]
fn seed_never_overwrites_a_newer_event() {
    let mut queued = job(JobState::Queued);
    let mut live = LiveJobs::default();
    queued.state = JobState::Processing;
    apply(&mut live, queued.clone());
    queued.state = JobState::Queued;
    live.seed(&[queued.clone()]);
    assert_eq!(live.job(queued.id).unwrap().state, JobState::Processing);
}

#[test]
fn recent_finished_is_newest_first_and_limited() {
    let jobs: Vec<Job> = [5.0, 1.0, 3.0, 9.0]
        .into_iter()
        .map(|minutes| finished(JobState::Done, 20.0, minutes))
        .collect();
    let mut all = jobs.clone();
    all.push(job(JobState::Queued));
    let live = LiveJobs::new(&all);
    assert_eq!(
        ids(&live.recent_finished(3)),
        [jobs[1].id, jobs[2].id, jobs[0].id]
    );
    assert_eq!(live.recent_finished(0).len(), 0);
    assert_eq!(live.recent_finished(10).len(), 4);
}

#[test]
fn busy_slots_count_queued_and_processing() {
    let live = LiveJobs::new(&[
        job(JobState::Queued),
        job(JobState::Processing),
        job(JobState::WaitingForSlot),
        job(JobState::Downloading),
        job(JobState::Done),
    ]);
    assert_eq!(live.busy_slots(), 2);
    assert_eq!(live.slots_summary(3), "2 of 3 slots busy");
}

#[test]
fn submitting_counts_jobs_whose_submit_is_out() {
    let mut sending = job(JobState::WaitingForSlot);
    sending.submit_attempt = Some(SubmitAttempt { is_idempotent: false });
    let mut paused_during_submit = job(JobState::Paused(PauseReason::ConnectionNeedsAttention));
    paused_during_submit.submit_attempt = Some(SubmitAttempt { is_idempotent: true });
    let live = LiveJobs::new(&[
        sending,
        paused_during_submit,
        job(JobState::WaitingForSlot),
        job(JobState::Queued),
    ]);
    assert_eq!(live.submitting_count(), 1);
}

#[test]
fn pause_and_resume_all_follow_what_they_would_change() {
    let submitted = LiveJobs::new(&[
        job(JobState::Processing),
        job(JobState::Paused(PauseReason::InsufficientFunds)),
    ]);
    assert!(!submitted.can_pause_all());
    assert!(!submitted.can_resume_all());
    let waiting = LiveJobs::new(&[
        job(JobState::WaitingForSlot),
        job(JobState::Paused(PauseReason::ByUser)),
    ]);
    assert!(waiting.can_pause_all());
    assert!(waiting.can_resume_all());
    assert!(LiveJobs::new(&[job(JobState::Paused(PauseReason::ConnectionNeedsAttention))]).can_pause_all());
}

#[test]
fn apply_reports_state_changes_but_not_progress() {
    let mut uploading = JobFixture {
        progress: 0.2,
        ..JobFixture::default()
    }
    .job(JobState::Uploading);
    let mut live = LiveJobs::default();
    assert!(live.apply(uploading.clone(), now(), UTC));
    uploading.progress = 0.6;
    assert!(!live.apply(uploading.clone(), now(), UTC));
    uploading.state = JobState::Queued;
    assert!(live.apply(uploading, now(), UTC));
}

#[test]
fn old_finished_keeps_the_newest_todays_and_active_ones() {
    // Today in UTC started 826 minutes before the fixture's `now`.
    let earlier: Vec<Job> = [2000.0, 3000.0, 4000.0, 5000.0]
        .into_iter()
        .map(|minutes| finished(JobState::Done, 6000.0, minutes))
        .collect();
    let today = finished(failed("Boom"), 900.0, 10.0);
    let active = created(JobState::Processing, 9000.0);
    let mut all = earlier.clone();
    all.extend([today, active]);
    let old = LiveJobs::old_finished(all.iter(), 2, now(), UTC);
    assert_eq!(old, HashSet::from([earlier[1].id, earlier[2].id, earlier[3].id]));
}

#[test]
fn old_finished_keeps_a_batch_whole() {
    let batch = Uuid::new_v4();
    let mut kept = finished(JobState::Done, 6000.0, 2000.0);
    kept.batch_id = Some(batch);
    let mut mate = finished(JobState::Done, 6000.0, 3000.0);
    mate.batch_id = Some(batch);
    let alone = finished(JobState::Done, 6000.0, 4000.0);
    let all = [kept, mate, alone.clone()];
    assert_eq!(
        LiveJobs::old_finished(all.iter(), 1, now(), UTC),
        HashSet::from([alone.id])
    );
}

#[test]
fn a_finishing_job_drops_old_finished_ones() {
    let limit = LiveJobs::STORED_FINISHED_LIMIT;
    let earlier: Vec<Job> = (0..limit + 2)
        .map(|index| {
            finished(
                JobState::Done,
                9000.0,
                2000.0 + f64::from(u32::try_from(index).unwrap()),
            )
        })
        .collect();
    let mut finishing = created(JobState::Downloading, 1.0);
    let mut all = earlier.clone();
    all.push(finishing.clone());
    let mut live = LiveJobs::new(&all);
    assert_eq!(live.recent_finished(usize::MAX).len(), limit + 2);
    finishing.state = JobState::Done;
    finishing.finished_at = Some(now());
    apply(&mut live, finishing.clone());
    let mut expected = vec![finishing.id];
    expected.extend(earlier.iter().take(limit - 1).map(|job| job.id));
    assert_eq!(ids(&live.recent_finished(usize::MAX)), expected);
}

// MARK: LiveJobsGenerationTimesTests

#[test]
fn history_and_finished_events_feed_the_estimate() {
    let mut live = LiveJobs::default();
    let spec = timed(JobState::Processing, 4.0, None).spec;
    live.add_history(&history(60.0, 2));
    assert_eq!(live.generation_times.estimate(&spec), None);
    apply(&mut live, timed(JobState::Done, 4.0, Some(90.0)));
    let estimate = live.generation_times.estimate(&spec).unwrap();
    assert!((estimate.processing - 60.0).abs() < 1e-9);
    assert_eq!(estimate.sample_count, 3);
}

#[test]
fn seeded_finished_jobs_count() {
    let live = LiveJobs::new(&history(45.0, 3));
    let spec = timed(JobState::Processing, 4.0, None).spec;
    assert!((live.generation_times.estimate(&spec).unwrap().processing - 45.0).abs() < 1e-9);
}

#[test]
fn active_jobs_add_nothing() {
    let mut live = LiveJobs::default();
    let processing = timed(JobState::Processing, 4.0, None);
    apply(&mut live, processing.clone());
    live.add_history(&[timed(JobState::Queued, 4.0, None)]);
    assert_eq!(live.generation_times.estimate(&processing.spec), None);
    assert!(!GenerationTimes::default().needs_recording(&processing));
}

// MARK: QueueProblemsTests

fn never() -> QueueProblems {
    QueueProblems::new(None, None)
}

#[test]
fn a_failed_job_stays_reachable_after_many_newer_results() {
    let failed_job = finished(failed("Timed out"), 60.0, 50.0);
    let done: Vec<Job> = (0..20)
        .map(|minutes| finished(JobState::Done, 40.0, f64::from(minutes)))
        .collect();
    let live = LiveJobs::new(&done);
    assert!(live.job(failed_job.id).is_none());
    assert_eq!(
        entry_ids(&live.problem_entries(std::slice::from_ref(&failed_job), &never())),
        [failed_job.id]
    );
    assert_eq!(live.recent_finished_entries(8).len(), 8);
    assert!(
        live.recent_finished_entries(usize::MAX)
            .iter()
            .all(|entry| entry.id() != failed_job.id)
    );
}

#[test]
fn lists_failed_rejected_and_canceled_newest_first() {
    let failed_job = finished(failed("x"), 3.0, 3.0);
    let rejected = finished(JobState::Rejected, 3.0, 1.0);
    let canceled = finished(JobState::Canceled, 3.0, 2.0);
    let done = finished(JobState::Done, 3.0, 0.0);
    let running = job(JobState::Processing);
    let live = LiveJobs::new(&[done.clone(), running, canceled.clone()]);
    let entries = live.problem_entries(&[failed_job.clone(), rejected.clone(), done.clone()], &never());
    assert_eq!(entry_ids(&entries), [rejected.id, canceled.id, failed_job.id]);
    assert_eq!(entry_ids(&live.recent_finished_entries(8)), [done.id]);
}

#[test]
fn a_retried_job_leaves_the_section_at_once() {
    let mut retried = finished(failed("x"), 3.0, 3.0);
    let stored = retried.clone();
    let mut live = LiveJobs::default();
    retried.state = JobState::Preparing;
    retried.finished_at = None;
    apply(&mut live, retried);
    assert_eq!(live.problem_entries(&[stored], &never()).len(), 0);
}

#[test]
fn a_batch_still_running_keeps_its_failed_job_in_active() {
    let batch = Uuid::new_v4();
    let mut failed_job = finished(failed("x"), 3.0, 1.0);
    failed_job.batch_id = Some(batch);
    let mut running = job(JobState::Processing);
    running.batch_id = Some(batch);
    let live = LiveJobs::new(&[failed_job.clone(), running]);
    assert_eq!(live.problem_entries(&[failed_job], &never()).len(), 0);
    assert_eq!(entry_ids(&live.active_entries()), [batch]);
}

#[test]
fn remove_and_clear_all_hide_jobs() {
    let older = finished(failed("x"), 30.0, 30.0);
    let newer = finished(JobState::Canceled, 10.0, 10.0);
    let mut live = LiveJobs::default();
    let mut problems = never();
    problems.remove(newer.id, &HashSet::from([older.id, newer.id]));
    let stored = [older.clone(), newer.clone()];
    assert_eq!(entry_ids(&live.problem_entries(&stored, &problems)), [older.id]);
    problems.clear_all(minutes_ago(20.0));
    assert!(problems.removed.is_empty());
    assert_eq!(entry_ids(&live.problem_entries(&stored, &problems)), [newer.id]);
    let later = finished(failed("y"), 1.0, 1.0);
    apply(&mut live, later.clone());
    // Failed after "Clear all", so it shows again.
    assert_eq!(
        entry_ids(&live.problem_entries(&stored, &problems)),
        [later.id, newer.id]
    );
}

#[test]
fn removing_forgets_ids_the_store_no_longer_returns() {
    let (gone, kept, new) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut problems = QueueProblems::new(None, Some(&format!("{gone},{kept}")));
    problems.remove(new, &HashSet::from([kept, new]));
    assert_eq!(problems.removed, HashSet::from([kept, new]));
}

#[test]
fn round_trips_through_its_stored_values() {
    let ids = [Uuid::new_v4(), Uuid::new_v4()];
    let mut problems = QueueProblems::new(Some(0.0), Some(""));
    assert_eq!(problems.cleared_at, None);
    assert_eq!(problems.stored_cleared_at(), None);
    problems.clear_all(now());
    let loaded = HashSet::from(ids);
    problems.remove(ids[0], &loaded);
    problems.remove(ids[1], &loaded);
    let restored = QueueProblems::new(problems.stored_cleared_at(), problems.stored_removed().as_deref());
    assert_eq!(restored, problems);
    assert!(QueueProblems::new(None, Some("not-an-id,,")).removed.is_empty());
}

// MARK: Batches

#[test]
fn a_batch_groups_under_one_header_with_its_progress() {
    let batch = Uuid::new_v4();
    let mut first = created(JobState::Done, 5.0);
    first.batch_id = Some(batch);
    let mut second = JobFixture {
        created_minutes_ago: 4.0,
        progress: 0.5,
        ..JobFixture::default()
    }
    .job(JobState::Uploading);
    second.batch_id = Some(batch);
    let mut third = created(failed("x"), 3.0);
    third.batch_id = Some(batch);
    let alone = created(JobState::Queued, 1.0);
    let live = LiveJobs::new(&[first, second, third, alone.clone()]);
    let entries = live.active_entries();
    assert_eq!(entry_ids(&entries), [batch, alone.id]);
    let QueueEntry::Batch(group) = &entries[0] else {
        panic!("expected a batch");
    };
    assert_eq!(group.title(), "dance · Motion transfer batch · 3 variants");
    assert_eq!(group.progress_text(), "1 of 3 done · 1 stopped");
    assert!((group.progress() - (1.0 + 1.5 / 4.0 + 1.0) / 3.0).abs() < 1e-9);
    assert_eq!(group.total_cost_text().as_deref(), Some("$16.34"));
}
