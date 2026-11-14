//! Run next and the times of a job's stages: a port of `RunNextTests`.

use jutsu_core::{FailureReason, Job, JobState, RemoteStatus};
use time::Duration;

use super::{Harness, eventually, failed, make_spec};
use crate::{EngineError, EnqueueOptions, JobEngine};

/// Queues `count` jobs a minute apart and waits until the engine holds them all.
async fn enqueue(count: usize, engine: &JobEngine, harness: &Harness) -> Vec<Job> {
    let mut jobs = Vec::new();
    for _ in 0..count {
        jobs.push(
            engine
                .enqueue(make_spec(1), EnqueueOptions::default())
                .await
                .unwrap(),
        );
        harness.clock.advance(Duration::seconds(60));
    }
    jobs
}

fn holding(max_concurrency: usize) -> Harness {
    let mut harness = Harness::new();
    harness.max_concurrency = max_concurrency;
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::Queued));
    harness
}

fn submit_keys(harness: &Harness) -> Vec<uuid::Uuid> {
    harness.provider.with(|state| state.submit_keys.clone())
}

#[tokio::test]
async fn run_next_on_the_last_of_five_waiting_jobs_submits_it_next() {
    let harness = holding(2);
    let engine = harness.engine().await;
    let running = enqueue(2, &engine, &harness).await;
    for job in &running {
        harness.repository.wait_reached(job.id, &JobState::Queued).await;
    }
    let waiting = enqueue(5, &engine, &harness).await;
    eventually(|| engine.limiter().waiter_count() == 5).await;

    let last = waiting.last().unwrap();
    engine.run_next(last.id).await.unwrap();
    assert!(harness.repository.stored(last.id).unwrap().promoted_at.is_some());
    engine.cancel(running[0].id).await.unwrap();

    harness.repository.wait_reached(last.id, &JobState::Queued).await;
    assert_eq!(submit_keys(&harness).len(), 3);
    assert_eq!(submit_keys(&harness).last(), Some(&last.idempotency_key));
    let expected: Vec<_> = waiting[..4].iter().map(|job| job.id).collect();
    assert_eq!(engine.limiter().waiting_ids(), expected);
}

#[tokio::test]
async fn waiting_order_and_promotion_survive_a_restart() {
    let harness = holding(1);
    let first = harness.engine().await;
    let running = enqueue(1, &first, &harness).await.remove(0);
    harness
        .repository
        .wait_reached(running.id, &JobState::Queued)
        .await;
    let waiting = enqueue(3, &first, &harness).await;
    eventually(|| first.limiter().waiter_count() == 3).await;
    first.run_next(waiting[1].id).await.unwrap();
    first.stop(Duration::ZERO).await;

    let second = harness.engine().await;
    eventually(|| second.limiter().waiter_count() == 3).await;
    assert_eq!(
        second.limiter().waiting_ids(),
        [waiting[1].id, waiting[0].id, waiting[2].id]
    );

    second.cancel(running.id).await.unwrap();
    harness
        .repository
        .wait_reached(waiting[1].id, &JobState::Queued)
        .await;
    assert_eq!(submit_keys(&harness).last(), Some(&waiting[1].idempotency_key));
}

#[tokio::test]
async fn run_next_only_reorders_jobs_waiting_for_a_slot() {
    let harness = holding(1);
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_reached(job.id, &JobState::Queued).await;

    assert_eq!(
        engine.run_next(job.id).await,
        Err(EngineError::NotWaiting(JobState::Queued))
    );
    assert_eq!(harness.repository.stored(job.id).unwrap().promoted_at, None);
}

#[tokio::test]
async fn stage_times_are_recorded_and_a_retry_clears_them() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [
            Ok(RemoteStatus::Queued),
            Ok(RemoteStatus::InProgress),
            Ok(RemoteStatus::InProgress),
        ]
        .into();
    });
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();

    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    let submitted_at = done.submitted_at.unwrap();
    let processing_at = done.processing_at.unwrap();
    let charged_at = done.charged_at.unwrap();
    assert!(job.created_at <= submitted_at);
    assert!(processing_at > submitted_at);
    assert!(charged_at > processing_at);
    assert_eq!(done.queue_wait(), Some(processing_at - job.created_at));
    assert_eq!(done.processing_time(), Some(charged_at - processing_at));

    harness.provider.with(|state| {
        state.status_steps = [
            Ok(RemoteStatus::InProgress),
            Ok(RemoteStatus::Failed {
                message: Some("boom".into()),
            }),
        ]
        .into();
    });
    let again = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let failed_job = harness
        .repository
        .wait_state(again.id, &failed(FailureReason::Message("boom".into())))
        .await;
    let first_submit = failed_job.submitted_at.unwrap();
    assert!(failed_job.processing_at.is_some());

    // The second attempt is never reported in progress.
    engine.retry(again.id).await.unwrap();
    let retried = harness.repository.wait_state(again.id, &JobState::Done).await;
    assert!(retried.submitted_at.unwrap() > first_submit);
    assert_eq!(retried.processing_at, None);
}
