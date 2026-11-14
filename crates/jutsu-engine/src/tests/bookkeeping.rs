//! Port of `EngineBookkeepingTests`: submits in flight, the shared status call's waiters and pace, write turns and
//! the network gate. A Swift continuation is a oneshot sender here; the waiting task is its receiver.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use jutsu_core::RemoteStatus;
use time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::EngineSettings;
use crate::bookkeeping::{BatchedStatus, NetworkGate, StatusBatch, StatusPace, SubmitTracker, WriteTurns};
use crate::system_random;

// SubmitTracker.

#[test]
fn no_submit_starts_while_stopping() {
    let mut submits = SubmitTracker::default();
    let id = Uuid::new_v4();
    submits.is_stopping = true;
    assert!(!submits.begin(id));
    assert!(submits.in_flight.is_empty());
    submits.is_stopping = false;
    assert!(submits.begin(id));
    assert_eq!(submits.in_flight, HashSet::from([id]));
}

#[tokio::test]
async fn the_last_answer_releases_the_stop() {
    let mut submits = SubmitTracker::default();
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    submits.begin(first);
    submits.begin(second);
    let (stop, stopped) = oneshot::channel();
    submits.wait(stop);
    submits.finish(first);
    assert_eq!(submits.waiter_count(), 1);
    submits.finish(second);
    assert_eq!(submits.waiter_count(), 0);
    stopped.await.unwrap();
    assert!(submits.in_flight.is_empty());
}

#[tokio::test]
async fn a_stop_with_nothing_out_returns_at_once() {
    let mut submits = SubmitTracker::default();
    let (stop, stopped) = oneshot::channel();
    submits.wait(stop);
    stopped.await.unwrap();
    assert_eq!(submits.waiter_count(), 0);
}

#[tokio::test]
async fn running_out_of_time_releases_the_stop_with_the_submit_still_out() {
    let mut submits = SubmitTracker::default();
    let id = Uuid::new_v4();
    submits.begin(id);
    assert!(!submits.release_waiters());
    let (stop, stopped) = oneshot::channel();
    submits.wait(stop);
    let released = submits.release_waiters();
    stopped.await.unwrap();
    assert!(released);
    assert_eq!(submits.in_flight, HashSet::from([id]));
}

// StatusBatch.

fn waiting(batch: &StatusBatch) -> HashSet<Uuid> {
    batch.waiting().into_iter().collect()
}

fn status(answer: Result<BatchedStatus, oneshot::error::RecvError>) -> Option<RemoteStatus> {
    match answer.unwrap() {
        BatchedStatus::Status(status) => Some(status),
        BatchedStatus::Alone => None,
    }
}

#[tokio::test]
async fn each_waiter_gets_its_own_answer_or_goes_alone() {
    let mut batch = StatusBatch::default();
    let (answered, missing) = (Uuid::new_v4(), Uuid::new_v4());
    let (first_sender, first) = oneshot::channel();
    let (second_sender, second) = oneshot::channel();
    batch.wait(answered, first_sender);
    batch.wait(missing, second_sender);
    assert_eq!(waiting(&batch), HashSet::from([answered, missing]));

    batch.answer(
        &[answered, missing],
        &HashMap::from([(answered, BatchedStatus::Status(RemoteStatus::InProgress))]),
    );

    assert_eq!(status(first.await), Some(RemoteStatus::InProgress));
    assert_eq!(status(second.await), None);
    assert!(batch.is_empty());
}

#[tokio::test]
async fn a_job_that_joins_during_the_call_waits_for_the_next_one() {
    let mut batch = StatusBatch::default();
    let (early, late) = (Uuid::new_v4(), Uuid::new_v4());
    let (first_sender, first) = oneshot::channel();
    batch.wait(early, first_sender);
    let asked = batch.waiting();
    let (second_sender, second) = oneshot::channel();
    batch.wait(late, second_sender);
    assert_eq!(waiting(&batch), HashSet::from([early, late]));

    batch.answer(
        &asked,
        &HashMap::from([(early, BatchedStatus::Status(RemoteStatus::Queued))]),
    );

    assert_eq!(status(first.await), Some(RemoteStatus::Queued));
    assert_eq!(waiting(&batch), HashSet::from([late]));
    batch.drop_waiter(late);
    assert!(second.await.is_err(), "a dropped waiter is cancelled");
}

#[tokio::test]
async fn a_dropped_waiter_is_cancelled_and_not_answered_again() {
    let mut batch = StatusBatch::default();
    let id = Uuid::new_v4();
    let (sender, waiter) = oneshot::channel();
    batch.wait(id, sender);

    batch.drop_waiter(id);
    batch.answer(
        &[id],
        &HashMap::from([(id, BatchedStatus::Status(RemoteStatus::Queued))]),
    );

    assert!(waiter.await.is_err(), "a dropped waiter is cancelled");
    assert!(batch.is_empty());
}

// StatusPace.

fn pace() -> StatusPace {
    let mut settings = EngineSettings::new(PathBuf::from("output"));
    settings.jitter = 0.0;
    StatusPace::new(&settings, system_random())
}

fn answer(job: Uuid, status: RemoteStatus) -> HashMap<Uuid, BatchedStatus> {
    HashMap::from([(job, BatchedStatus::Status(status))])
}

#[test]
fn waits_grow_until_a_status_changes() {
    let job = Uuid::new_v4();
    let mut pace = pace();
    assert_eq!(pace.delay, Duration::seconds(2));
    pace.update(&HashMap::from([(job, BatchedStatus::Alone)]), None);
    assert_eq!(pace.delay, Duration::seconds(3));
    pace.update(&answer(job, RemoteStatus::Queued), None);
    assert_eq!(pace.delay, Duration::seconds(2));
    pace.update(&answer(job, RemoteStatus::Queued), None);
    assert_eq!(pace.delay, Duration::seconds(3));
    pace.update(&answer(job, RemoteStatus::InProgress), None);
    assert_eq!(pace.delay, Duration::seconds(2));
}

#[test]
fn a_job_left_out_of_an_answer_is_forgotten() {
    let job = Uuid::new_v4();
    let mut pace = pace();
    pace.update(&answer(job, RemoteStatus::Queued), None);
    pace.update(&HashMap::new(), None);
    assert_eq!(pace.delay, Duration::seconds(3));
    pace.update(&answer(job, RemoteStatus::Queued), None);
    assert_eq!(pace.delay, Duration::seconds(2));
}

/// Swift's last step passes an infinite wait, which counts as none; a `Duration` can't be infinite, so it is `None`.
#[test]
fn the_wait_the_provider_asks_for_is_kept_in_range() {
    let mut pace = pace();
    pace.update(&HashMap::new(), Some(Duration::ZERO));
    assert_eq!(pace.delay, Duration::seconds(1));
    pace.update(&HashMap::new(), Some(Duration::seconds(7)));
    assert_eq!(pace.delay, Duration::seconds(7));
    pace.update(&HashMap::new(), Some(Duration::seconds(3600)));
    assert_eq!(pace.delay, Duration::seconds(30));
    pace.update(&HashMap::new(), None);
    assert_eq!(pace.delay, Duration::seconds(3));
}

// WriteTurns.

#[tokio::test]
async fn one_writer_per_job_at_a_time() {
    let mut turns = WriteTurns::default();
    let (job, other) = (Uuid::new_v4(), Uuid::new_v4());
    assert_eq!(
        [turns.take(job), turns.take(job), turns.take(other)],
        [true, false, true]
    );
    let (sender, waiter) = oneshot::channel();
    turns.wait(job, sender);
    assert!(turns.has_waiters(job));
    assert!(!turns.has_waiters(other));
    turns.end(job);
    waiter.await.unwrap();
    assert!(!turns.has_waiters(job));
    assert_eq!([turns.take(job), turns.take(other)], [true, false]);
}

// NetworkGate.

#[tokio::test]
async fn a_cancelled_waiter_leaves_alone_and_the_rest_wait_for_the_gate() {
    let mut gate = NetworkGate::default();
    let (cancelled, other) = (Uuid::new_v4(), Uuid::new_v4());
    let (first_sender, first) = oneshot::channel();
    let (second_sender, mut second) = oneshot::channel();
    gate.wait(cancelled, first_sender);
    gate.wait(other, second_sender);

    gate.open_one(cancelled);
    first.await.unwrap();
    assert!(second.try_recv().is_err(), "the other waiter still waits");

    gate.open();
    second.await.unwrap();
}
