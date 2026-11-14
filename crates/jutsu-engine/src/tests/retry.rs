//! Steps that fail for a reason waiting can fix, and the retry each one shows: a port of `RetryTests`.

use jutsu_core::{FailureReason, JobState, PauseReason, PortError, ProviderError, RetryCause, RetryStatus};
use time::Duration;
use tokio::sync::broadcast::Receiver;
use uuid::Uuid;

use super::{Harness, SubmitStep, failed, make_spec, paused, quiesce};
use crate::EnqueueOptions;
use crate::clock::EngineClock;
use crate::engine::EngineEvent;

/// Every distinct retry the engine published for `id`, until the job reaches `state`.
async fn retries(id: Uuid, events: &mut Receiver<EngineEvent>, until: &JobState) -> Vec<RetryStatus> {
    let mut retries: Vec<RetryStatus> = Vec::new();
    loop {
        let EngineEvent::JobChanged(job) = events.recv().await.unwrap() else {
            continue;
        };
        if job.id != id {
            continue;
        }
        if let Some(retrying) = job.retrying
            && retries.last() != Some(&retrying)
        {
            retries.push(retrying);
        }
        if job.state == *until {
            return retries;
        }
    }
}

fn provider(error: ProviderError) -> PortError {
    PortError::Provider(error)
}

fn busy(retry_after: Option<Duration>) -> SubmitStep {
    SubmitStep::Fail(provider(ProviderError::ConcurrencyLimit { retry_after }))
}

#[tokio::test]
async fn a_retry_shows_on_the_job_until_the_step_gets_through() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.upload_errors = [
            provider(ProviderError::Network),
            provider(ProviderError::Server { status: 503 }),
        ]
        .into();
    });
    let engine = harness.engine().await;
    let mut events = engine.subscribe();

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let retries = retries(job.id, &mut events, &JobState::Done).await;

    assert_eq!(
        retries.iter().map(|retry| retry.failures).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        retries.iter().map(|retry| retry.cause).collect::<Vec<_>>(),
        [RetryCause::Network, RetryCause::Server { status: 503 }]
    );
    let since = retries[0].since;
    assert!(retries.iter().all(|retry| retry.since == since));
    assert_eq!(
        retries
            .iter()
            .map(|retry| retry.next_attempt_at - since)
            .collect::<Vec<_>>(),
        [Duration::seconds(5), Duration::seconds(15)]
    );
    assert_eq!(harness.repository.stored(job.id).unwrap().retrying, None);
    assert_eq!(harness.log.count("upload"), 4);
}

#[tokio::test]
async fn a_step_that_gets_through_starts_the_count_and_the_backoff_again() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.upload_errors = [provider(ProviderError::Network), provider(ProviderError::Network)].into();
        state.status_steps = [Err(provider(ProviderError::Server { status: 502 }))].into();
    });
    let engine = harness.engine().await;
    let mut events = engine.subscribe();

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let retries = retries(job.id, &mut events, &JobState::Done).await;

    assert_eq!(
        retries.iter().map(|retry| retry.failures).collect::<Vec<_>>(),
        [1, 2, 1]
    );
    assert_eq!(
        retries.iter().map(|retry| retry.cause).collect::<Vec<_>>(),
        [
            RetryCause::Network,
            RetryCause::Network,
            RetryCause::Server { status: 502 }
        ]
    );
    let poll = retries.last().unwrap();
    assert_eq!(poll.next_attempt_at - poll.since, Duration::seconds(5));
}

#[tokio::test]
async fn a_job_that_keeps_failing_before_submit_pauses_until_resumed() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.upload_errors = std::iter::repeat_with(|| provider(ProviderError::ModelUnavailable))
            .take(200)
            .collect();
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let held = harness
        .repository
        .wait_state(job.id, &paused(PauseReason::KeptFailing))
        .await;

    let retrying = held.retrying.unwrap();
    assert_eq!(retrying.cause, RetryCause::ModelUnavailable);
    assert!(retrying.failures >= 5);
    assert!(harness.clock.now() - retrying.since >= Duration::minutes(15));
    assert!(harness.provider.with(|state| state.submit_keys.is_empty()));
    let uploads = harness.log.count("upload");
    quiesce().await;
    assert_eq!(harness.log.count("upload"), uploads);

    harness.provider.with(|state| state.upload_errors.clear());
    engine.resume(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(done.retrying, None);
}

#[tokio::test]
async fn a_busy_account_is_waited_for_past_the_pause_bound() {
    for retry_after in [None, Some(Duration::seconds(90))] {
        let harness = Harness::new();
        harness.provider.with(|state| {
            state.submit_steps = std::iter::repeat_with(|| busy(retry_after)).take(25).collect();
        });
        let engine = harness.engine().await;
        let start = harness.clock.now();

        let job = engine
            .enqueue(make_spec(1), EnqueueOptions::default())
            .await
            .unwrap();
        harness.repository.wait_state(job.id, &JobState::Done).await;

        assert!(
            harness.clock.now() - start > Duration::minutes(15),
            "{retry_after:?}"
        );
        assert!(
            !harness
                .repository
                .history(job.id)
                .contains(&paused(PauseReason::KeptFailing))
        );
        assert_eq!(
            harness.provider.with(|state| state.submit_keys.clone()),
            vec![job.idempotency_key; 26]
        );
    }
}

/// A full queue's `Retry-After` is the wait, in place of the backoff's, kept within `server_retry_delay`.
#[tokio::test]
async fn a_full_queue_waits_as_long_as_the_server_asks() {
    for (retry_after, wait) in [(30, 30), (0, 1), (86_400, 600)] {
        let harness = Harness::new();
        harness
            .provider
            .with(|state| state.submit_steps = [busy(Some(Duration::seconds(retry_after)))].into());
        let engine = harness.engine().await;
        let mut events = engine.subscribe();

        let job = engine
            .enqueue(make_spec(1), EnqueueOptions::default())
            .await
            .unwrap();
        let retries = retries(job.id, &mut events, &JobState::Done).await;

        assert_eq!(
            retries.iter().map(|retry| retry.cause).collect::<Vec<_>>(),
            [RetryCause::ConcurrencyLimit]
        );
        let retry = retries[0];
        assert_eq!(
            retry.next_attempt_at - retry.since,
            Duration::seconds(wait),
            "{retry_after}"
        );
    }
}

#[tokio::test]
async fn a_submitted_job_whose_polls_keep_failing_is_never_paused_and_fails_at_the_deadline() {
    let mut harness = Harness::new();
    harness.generation_deadline = Duration::hours(1);
    harness.provider.with(|state| {
        state.status_steps = std::iter::repeat_with(|| Err(provider(ProviderError::Network)))
            .take(500)
            .collect();
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let failed_job = harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Deadline))
        .await;

    assert!(failed_job.timed_out_at.is_some());
    assert!(failed_job.may_still_finish());
    assert_eq!(failed_job.retrying, None);
    assert_eq!(harness.provider.with(|state| state.submit_keys.len()), 1);
    assert!(
        !harness
            .repository
            .history(job.id)
            .contains(&paused(PauseReason::KeptFailing))
    );
    assert_eq!(engine.limiter().in_use(), 0);
}

#[tokio::test]
async fn a_restart_forgets_the_retries_of_the_run_before() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.upload_errors = std::iter::repeat_with(|| provider(ProviderError::Network))
            .take(3)
            .collect();
    });
    let first = harness.engine().await;
    let job = first
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_until(|repository| {
            repository
                .stored(job.id)
                .is_some_and(|job| job.retrying.is_some())
        })
        .await;
    first.stop(Duration::ZERO).await;
    harness.provider.with(|state| state.upload_errors.clear());

    let second = harness.engine().await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(done.retrying, None);
    second.stop(Duration::ZERO).await;
}
