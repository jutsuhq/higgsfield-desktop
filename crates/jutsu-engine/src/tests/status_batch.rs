//! Polling a connection that reads several statuses in one call, as the account's `jobs_wait` does: a port of
//! `StatusBatchTests`.

use std::collections::{HashMap, HashSet};

use jutsu_core::{FailureReason, Job, JobState, PortError, ProviderError, Rejection, RemoteStatus};
use time::Duration;

use super::{BatchMode, Gate, Harness, eventually, failed, make_spec, quiesce};
use crate::bookkeeping::StatusBatch;
use crate::{EnqueueOptions, JobEngine};

async fn enqueue(count: usize, engine: &JobEngine, harness: &Harness) -> Vec<Job> {
    let mut jobs = Vec::new();
    for _ in 0..count {
        jobs.push(
            engine
                .enqueue(make_spec(1), EnqueueOptions::default())
                .await
                .unwrap(),
        );
        harness.clock.advance(Duration::seconds(1));
    }
    jobs
}

fn batching(max_concurrency: usize) -> Harness {
    let mut harness = Harness::new();
    harness.max_concurrency = max_concurrency;
    harness.provider.with(|state| state.status_batch_limit = 12);
    harness
}

fn hold(harness: &Harness, status: Option<RemoteStatus>) {
    harness.provider.with(|state| state.held_status = status);
}

fn batches(harness: &Harness) -> Vec<Vec<String>> {
    harness.provider.with(|state| state.batch_calls.clone())
}

async fn processing(harness: &Harness, count: usize) {
    harness
        .repository
        .wait_until(|repository| {
            repository
                .all()
                .iter()
                .filter(|job| job.state == JobState::Processing)
                .count()
                == count
        })
        .await;
}

fn request_ids(range: std::ops::RangeInclusive<usize>) -> HashSet<String> {
    range.map(|index| format!("r{index}")).collect()
}

#[test]
fn groups_hold_at_most_the_limit() {
    let all: Vec<i32> = (0..26).collect();
    assert_eq!(
        StatusBatch::groups(&all, 12),
        [(0..12).collect::<Vec<_>>(), (12..24).collect(), vec![24, 25]]
    );
    let twelve: Vec<i32> = (0..12).collect();
    assert_eq!(StatusBatch::groups(&twelve, 12), std::slice::from_ref(&twelve));
    assert_eq!(StatusBatch::groups::<i32>(&[], 12), [] as [Vec<i32>; 0]);
    assert_eq!(StatusBatch::groups(&[1, 2], 0), [vec![1], vec![2]]);
}

#[tokio::test]
async fn six_processing_jobs_share_one_status_call_per_tick() {
    let harness = batching(6);
    hold(&harness, Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    let jobs = enqueue(6, &engine, &harness).await;
    processing(&harness, 6).await;
    let all = request_ids(1..=6);
    eventually(|| {
        batches(&harness)
            .iter()
            .any(|batch| batch.iter().cloned().collect::<HashSet<_>>() == all)
    })
    .await;

    hold(&harness, None);
    for job in &jobs {
        harness.repository.wait_reached(job.id, &JobState::Done).await;
    }
    assert_eq!(harness.log.count("status"), 0);
    let batches = batches(&harness);
    assert!(batches.iter().all(|batch| {
        !batch.is_empty() && batch.len() <= 12 && batch.iter().collect::<HashSet<_>>().len() == batch.len()
    }));
    // One call per tick: each call waits out a tick of its own.
    assert!(batches.len() <= harness.clock.sleeps().len());
}

#[tokio::test]
async fn more_than_twelve_jobs_are_split_into_groups() {
    let harness = batching(14);
    hold(&harness, Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    enqueue(14, &engine, &harness).await;
    processing(&harness, 14).await;

    let all = request_ids(1..=14);
    eventually(|| {
        let batches = batches(&harness);
        batches.windows(2).any(|pair| {
            pair[0].len() == 12 && pair[0].iter().chain(&pair[1]).cloned().collect::<HashSet<_>>() == all
        })
    })
    .await;
    assert!(batches(&harness).iter().all(|batch| batch.len() <= 12));
    assert_eq!(harness.log.count("status"), 0);
}

#[tokio::test]
async fn each_job_gets_its_own_status_from_the_shared_answer() {
    let harness = batching(3);
    harness.provider.with(|state| {
        state.batch_statuses = HashMap::from([
            (
                "r2".into(),
                RemoteStatus::Failed {
                    message: Some("Bad input".into()),
                },
            ),
            ("r3".into(), RemoteStatus::Rejected(Rejection::ExplicitContent)),
        ]);
    });
    let engine = harness.engine().await;
    let jobs = enqueue(3, &engine, &harness).await;

    harness
        .repository
        .wait_until(|repository| {
            jobs.iter().all(|job| {
                repository
                    .stored(job.id)
                    .is_some_and(|job| !job.state.is_active())
            })
        })
        .await;
    let states: HashMap<String, JobState> = jobs
        .iter()
        .map(|job| {
            let stored = harness.repository.stored(job.id).unwrap();
            (stored.remote.unwrap().request_id, stored.state)
        })
        .collect();
    assert_eq!(states["r1"], JobState::Done);
    assert_eq!(states["r2"], failed(FailureReason::Message("Bad input".into())));
    assert_eq!(states["r3"], JobState::Rejected);
    assert_eq!(harness.log.count("status"), 0);
}

/// The clamp itself is `bookkeeping::the_wait_the_provider_asks_for_is_kept_in_range`; this checks the shared call
/// hands the provider's wait to it.
#[tokio::test]
async fn the_wait_the_provider_asks_for_is_kept_within_bounds() {
    let harness = batching(2);
    hold(&harness, Some(RemoteStatus::InProgress));
    harness
        .provider
        .with(|state| state.batch_poll_after = Some(Duration::seconds(3600)));
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    eventually(|| batches(&harness).len() >= 3).await;

    let sleeps = harness.clock.sleeps();
    assert!(sleeps.contains(&Duration::seconds(30)), "{sleeps:?}");
    assert!(!sleeps.contains(&Duration::seconds(3600)), "{sleeps:?}");
    hold(&harness, None);
    harness.repository.wait_reached(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn without_a_shared_answer_each_job_asks_alone() {
    let modes = [
        || BatchMode::Unavailable,
        || BatchMode::Fail(PortError::Provider(ProviderError::Network)),
        || {
            BatchMode::Fail(PortError::Provider(ProviderError::Validation {
                field: None,
                message: "x".into(),
            }))
        },
    ];
    for mode in modes {
        let harness = batching(2);
        harness.provider.with(|state| state.batch_mode = mode());
        hold(&harness, Some(RemoteStatus::InProgress));
        let engine = harness.engine().await;
        let jobs = enqueue(2, &engine, &harness).await;
        processing(&harness, 2).await;
        eventually(|| harness.log.count("status") >= 4).await;

        hold(&harness, None);
        for job in &jobs {
            harness.repository.wait_reached(job.id, &JobState::Done).await;
        }
        assert_ne!(batches(&harness), [] as [Vec<String>; 0]);
    }
}

#[tokio::test]
async fn a_job_missing_from_the_answer_asks_alone() {
    let harness = batching(2);
    harness
        .provider
        .with(|state| state.batch_omits = HashSet::from(["r1".into()]));
    hold(&harness, Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    let jobs = enqueue(2, &engine, &harness).await;
    processing(&harness, 2).await;
    eventually(|| harness.log.count("status") >= 2).await;

    hold(&harness, None);
    for job in &jobs {
        harness.repository.wait_reached(job.id, &JobState::Done).await;
    }
    assert!(
        harness
            .provider
            .with(|state| state.status_asked.iter().all(|id| id == "r1"))
    );
}

#[tokio::test]
async fn cancel_stops_a_job_waiting_for_the_shared_call() {
    let harness = batching(2);
    let gate = Gate::new();
    harness
        .provider
        .with(|state| state.batch_mode = BatchMode::Held(gate.clone()));
    hold(&harness, Some(RemoteStatus::Queued));
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    eventually(|| gate.is_waiting()).await;

    engine.cancel(job.id).await.unwrap();

    assert_eq!(
        harness.repository.stored(job.id).unwrap().state,
        JobState::Canceled
    );
    assert_eq!(harness.provider.with(|state| state.cancels.clone()), ["r1"]);
    gate.open();
    quiesce().await;
    assert_eq!(
        harness.repository.history(job.id).last(),
        Some(&JobState::Canceled)
    );
    assert_eq!(harness.log.count("status"), 0);
}

#[tokio::test]
async fn stop_does_not_wait_for_the_shared_call() {
    let harness = batching(2);
    let gate = Gate::new();
    harness
        .provider
        .with(|state| state.batch_mode = BatchMode::Held(gate.clone()));
    hold(&harness, Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    eventually(|| gate.is_waiting()).await;

    engine.stop(Duration::ZERO).await;

    assert_eq!(harness.repository.stored(job.id).unwrap().state, JobState::Queued);
    gate.open();
    hold(&harness, None);
    harness
        .provider
        .with(|state| state.batch_mode = BatchMode::Answer);
    engine.start().await.unwrap();
    harness.repository.wait_reached(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn an_api_key_still_polls_each_job() {
    let mut harness = Harness::new();
    harness.max_concurrency = 2;
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::InProgress), Ok(RemoteStatus::InProgress)].into();
    });
    let engine = harness.engine().await;
    let jobs = enqueue(2, &engine, &harness).await;
    for job in &jobs {
        harness.repository.wait_reached(job.id, &JobState::Done).await;
    }
    assert_eq!(batches(&harness), [] as [Vec<String>; 0]);
    assert_eq!(harness.log.count("status"), 4);
}
