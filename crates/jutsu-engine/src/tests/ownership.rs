//! Jobs, uploads and submits belong to the connection that made them; downloads retried, stops and interleaved
//! writes: a port of `OwnershipTests` (with its `SubmitAttemptTests`, `DownloadRetryTests`, `InterleavedWriteTests`
//! and `StopTests` suites).

use std::collections::HashSet;
use std::sync::Arc;

use jutsu_core::{
    FailureReason, Job, JobState, PauseReason, PortError, ProviderError, ProviderKind, RemoteAsset,
    RemoteStatus, SubmitAttempt, UploadCache,
};
use time::Duration;
use url::Url;
use uuid::Uuid;

use super::{
    CallLog, DownloadMode, FakeProvider, Gate, Harness, SubmitStep, eventually, failed, make_spec, paused,
    quiesce, usd,
};
use crate::clock::EngineClock;
use crate::{EngineError, EnqueueOptions, JobEngine};

const IDEMPOTENT: Option<SubmitAttempt> = Some(SubmitAttempt { is_idempotent: true });
const NOT_IDEMPOTENT: Option<SubmitAttempt> = Some(SubmitAttempt { is_idempotent: false });

/// A harness whose provider is the API key "key-a", as Swift's fake is by default.
fn harness() -> Harness {
    let harness = Harness::new();
    harness.provider.with(|state| state.owner = Some("key-a".into()));
    harness
}

fn harness_with(download: DownloadMode) -> Harness {
    let mut harness = harness();
    harness.downloader.mode = download;
    harness
}

/// Another API key, with its own log.
fn other_account() -> FakeProvider {
    let other = FakeProvider::new(CallLog::default());
    other.with(|state| state.owner = Some("key-b".into()));
    other
}

fn cache_key(content_hash: &str, owner: &str) -> String {
    crate::engine::upload_cache_key(content_hash, ProviderKind::ApiKey, Some(owner))
}

fn stored_job(harness: &Harness, kind: ProviderKind, owner: Option<&str>) -> Job {
    let mut job = Job::new(
        Uuid::new_v4(),
        make_spec(1),
        kind,
        usd("1.30"),
        Uuid::new_v4(),
        harness.clock.now(),
    );
    job.owner = owner.map(Into::into);
    job
}

async fn enqueue(engine: &JobEngine) -> Job {
    engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap()
}

fn submit_keys(harness: &Harness) -> Vec<Uuid> {
    harness.provider.with(|state| state.submit_keys.clone())
}

fn stored(harness: &Harness, id: Uuid) -> Job {
    harness.repository.stored(id).unwrap()
}

fn times_in(harness: &Harness, id: Uuid, state: &JobState) -> usize {
    harness
        .repository
        .history(id)
        .iter()
        .filter(|saved| *saved == state)
        .count()
}

#[tokio::test]
async fn jobs_belong_to_the_connection_that_queued_them() {
    let harness = harness();
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;

    assert_eq!(job.owner.as_deref(), Some("key-a"));
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(done.owner.as_deref(), Some("key-a"));
}

#[tokio::test]
async fn another_account_of_the_same_kind_never_runs_unsubmitted_jobs() {
    let harness = harness();
    harness.connectivity.set(false);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_reached(job.id, &JobState::Uploading)
        .await;

    let other = other_account();
    engine.set_provider(Some(Arc::new(other.clone()))).await;
    let attention = paused(PauseReason::ConnectionNeedsAttention);
    assert_eq!(stored(&harness, job.id).state, attention);

    harness.connectivity.set(true);
    engine.resume(job.id).await.unwrap();
    harness
        .repository
        .wait_until(|_| times_in(&harness, job.id, &attention) == 2)
        .await;
    assert!(other.with(|state| state.uploads.is_empty()));
    assert!(other.with(|state| state.submit_keys.is_empty()));

    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_state(job.id, &JobState::Done).await;
    assert!(other.with(|state| state.uploads.is_empty()));
}

#[tokio::test]
async fn submitted_jobs_are_not_polled_or_canceled_through_another_account() {
    let harness = harness();
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::Queued));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_reached(job.id, &JobState::Queued).await;

    let other = other_account();
    engine.set_provider(Some(Arc::new(other.clone()))).await;
    quiesce().await;
    assert_eq!(engine.cancel(job.id).await, Err(EngineError::NotConnected));
    assert_eq!(other.log.count("status"), 0);
    assert!(other.with(|state| state.cancels.is_empty()));
    assert_eq!(stored(&harness, job.id).state, JobState::Queued);

    harness.provider.with(|state| state.held_status = None);
    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn legacy_jobs_adopt_the_connection_restored_at_launch() {
    let harness = harness();
    let legacy = stored_job(&harness, ProviderKind::ApiKey, None);
    harness.repository.put(legacy.clone());

    let _engine = harness.engine().await;

    let done = harness.repository.wait_state(legacy.id, &JobState::Done).await;
    assert_eq!(done.owner.as_deref(), Some("key-a"));
}

#[tokio::test]
async fn legacy_jobs_of_another_kind_are_not_adopted() {
    let harness = harness();
    let legacy = stored_job(&harness, ProviderKind::Account, None);
    harness.repository.put(legacy.clone());

    let _engine = harness.engine().await;

    let held = harness
        .repository
        .wait_state(legacy.id, &paused(PauseReason::ConnectionNeedsAttention))
        .await;
    assert_eq!(held.owner, None);
    assert!(harness.provider.with(|state| state.uploads.is_empty()));
}

#[tokio::test]
async fn uploads_are_reused_only_by_the_account_that_made_them() {
    let harness = harness();
    let foreign = RemoteAsset::new(Url::parse("https://upload.example/other-account").unwrap());
    harness
        .cache
        .store(&foreign, &cache_key("video", "key-b"), harness.clock.now())
        .await
        .unwrap();
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert!(
        harness
            .provider
            .with(|state| state.uploads.contains(&"video".to_owned()))
    );
    assert_ne!(done.inputs.unwrap().video, Some(foreign));
}

// SubmitAttemptTests.

#[tokio::test]
async fn refused_submit_without_idempotency_is_tried_again() {
    let harness = harness();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(
            ProviderError::ConcurrencyLimit { retry_after: None },
        ))]
        .into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(submit_keys(&harness).len(), 2);
    assert_eq!(done.submit_attempt, None);
}

#[tokio::test]
async fn cancel_during_a_submit_without_idempotency_fails_as_unknown() {
    let harness = harness();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Hang].into();
    });
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    engine.cancel(job.id).await.unwrap();

    assert_eq!(
        stored(&harness, job.id).state,
        failed(FailureReason::OutcomeUnknown)
    );
    eventually(|| engine.limiter().in_use() == 0).await;
    assert_eq!(submit_keys(&harness).len(), 1);
}

#[tokio::test]
async fn cancel_during_an_idempotent_submit_is_refused_and_the_submit_goes_on() {
    let harness = harness();
    harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Hang].into());
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    assert_eq!(engine.cancel(job.id).await, Err(EngineError::SubmitInProgress));

    let saved = stored(&harness, job.id);
    assert_eq!(saved.state, JobState::WaitingForSlot);
    assert_eq!(saved.submit_attempt, IDEMPOTENT);
    assert!(engine.has_runner(job.id));
    engine.stop(Duration::ZERO).await;
}

/// A submit answered while a cancel stops its task keeps the paid request: the cancel then reaches the server instead
/// of failing the job as may-have-started.
///
/// Swift opens the gate after the cancel took the job's task, which keeps awaiting the fake's answer; a Rust task's
/// future is dropped once it is cancelled, so here the answer arrives as the cancel lands, with no turn in between.
#[tokio::test]
async fn a_submit_answered_during_a_cancel_keeps_its_request() {
    let harness = harness();
    let answer = Gate::new();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Answer(answer.clone())].into();
        state.held_status = Some(RemoteStatus::Queued);
    });
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    answer.open();
    engine.cancel(job.id).await.unwrap();

    let saved = stored(&harness, job.id);
    assert_eq!(saved.state, JobState::Canceled);
    assert_eq!(saved.remote.unwrap().request_id, "r1");
    assert_eq!(harness.provider.with(|state| state.cancels.clone()), ["r1"]);
}

/// A submit with the key whose answer was lost may be running: the job keeps resending it with that key, and can't
/// be canceled as never sent, even once it pauses for failing.
#[tokio::test]
async fn a_lost_answer_keeps_the_submit_marker() {
    let harness = harness();
    harness.provider.with(|state| {
        state.submit_steps =
            std::iter::repeat_with(|| SubmitStep::Fail(PortError::Provider(ProviderError::Network)))
                .take(100)
                .collect();
    });
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;

    let held = harness
        .repository
        .wait_state(job.id, &paused(PauseReason::KeptFailing))
        .await;

    assert_eq!(held.submit_attempt, IDEMPOTENT);
    let keys: HashSet<Uuid> = submit_keys(&harness).into_iter().collect();
    assert_eq!(keys, HashSet::from([job.idempotency_key]));
    assert_eq!(engine.cancel(job.id).await, Err(EngineError::SubmitInProgress));
    engine.stop(Duration::ZERO).await;
}

/// The pause cuts the submit off; the job keeps its marker, so the resend after the pause uses the same key.
async fn pause_during_an_idempotent_submit(harness: &Harness) -> (JobEngine, Job) {
    harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Hang].into());
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(harness).len() == 1).await;
    engine.set_provider(Some(Arc::new(other_account()))).await;
    let held = stored(harness, job.id);
    assert_eq!(held.state, paused(PauseReason::ConnectionNeedsAttention));
    assert_eq!(held.submit_attempt, IDEMPOTENT);
    (engine, job)
}

#[tokio::test]
async fn connection_switch_during_a_submit_resends_it_with_its_key_on_return() {
    let harness = harness();
    let (engine, job) = pause_during_an_idempotent_submit(&harness).await;

    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(submit_keys(&harness), [job.idempotency_key, job.idempotency_key]);
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 1);
    assert_eq!(done.submit_attempt, None);
}

#[tokio::test]
async fn cancel_of_a_job_paused_during_its_submit_waits_for_its_connection() {
    let harness = harness();
    let (engine, job) = pause_during_an_idempotent_submit(&harness).await;

    assert_eq!(engine.cancel(job.id).await, Err(EngineError::SubmitInProgress));

    assert_eq!(
        stored(&harness, job.id).state,
        paused(PauseReason::ConnectionNeedsAttention)
    );
    assert_eq!(submit_keys(&harness).len(), 1);
}

#[tokio::test]
async fn retry_after_an_unknown_outcome_resends_the_same_key() {
    let harness = harness();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(
            ProviderError::OutcomeUnknown,
        ))]
        .into();
    });
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::OutcomeUnknown))
        .await;

    engine.retry(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.idempotency_key, job.idempotency_key);
    assert_eq!(submit_keys(&harness), [job.idempotency_key, job.idempotency_key]);
}

// DownloadRetryTests.

#[tokio::test]
async fn retry_after_a_failed_download_fetches_the_paid_result_without_generating_again() {
    let harness = harness_with(DownloadMode::Refuse(vec![404]));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    let failed_job = harness
        .repository
        .wait_state(job.id, &failed(FailureReason::OutputUnavailable))
        .await;
    assert!(failed_job.has_paid_output());

    engine.retry(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 1);
    assert_eq!(done.idempotency_key, job.idempotency_key);
    assert_eq!(done.remote, failed_job.remote);
    assert_eq!(done.charged_at, failed_job.charged_at);
    assert_eq!(harness.log.count("download"), 2);
    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"video");
}

#[tokio::test]
async fn retry_after_a_canceled_download_downloads_again() {
    let harness = harness_with(DownloadMode::Hang);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_reached(job.id, &JobState::Downloading)
        .await;
    engine.cancel(job.id).await.unwrap();

    engine.retry(job.id).await.unwrap();
    harness
        .repository
        .wait_until(|repository| {
            repository
                .history(job.id)
                .iter()
                .skip_while(|state| **state != JobState::Canceled)
                .any(|state| *state == JobState::Downloading)
        })
        .await;

    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
    assert_eq!(harness.log.count("prepareVideo"), 1);
    engine.stop(Duration::ZERO).await;
}

#[tokio::test]
async fn expired_result_fails_with_a_clear_message() {
    let harness = harness_with(DownloadMode::Refuse(vec![404]));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    let reason = failed(FailureReason::OutputUnavailable);
    harness.repository.wait_reached(job.id, &reason).await;
    harness.provider.with(|state| {
        state.status_steps = [Err(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "Not found".into(),
        }))]
        .into();
    });

    engine.retry(job.id).await.unwrap();
    harness
        .repository
        .wait_until(|_| times_in(&harness, job.id, &reason) == 2)
        .await;

    assert_eq!(harness.log.count("download"), 1);
    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
}

#[tokio::test]
async fn a_throttled_download_is_retried_and_resumed() {
    for status in [408, 429] {
        let harness = harness_with(DownloadMode::Refuse(vec![status]));
        let engine = harness.engine().await;

        let job = enqueue(&engine).await;
        let done = harness.repository.wait_state(job.id, &JobState::Done).await;

        assert_eq!(harness.log.count("download"), 2, "{status}");
        assert!(harness.clock.sleeps().contains(&Duration::seconds(5)), "{status}");
        assert!(done.output_file.is_some());
    }
}

#[tokio::test]
async fn a_refusal_of_a_fresh_result_asks_for_a_new_url() {
    let harness = harness_with(DownloadMode::Refuse(vec![403]));
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("download"), 2);
    assert_eq!(harness.log.count("status"), 2);
}

#[tokio::test]
async fn a_refusal_of_a_result_older_than_its_lifetime_means_it_expired() {
    let harness = harness_with(DownloadMode::Refuse(vec![404, 403]));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    let reason = failed(FailureReason::OutputUnavailable);
    harness.repository.wait_reached(job.id, &reason).await;
    harness
        .clock
        .advance(crate::engine::RESULT_LIFETIME + Duration::seconds(60));

    engine.retry(job.id).await.unwrap();
    harness
        .repository
        .wait_until(|_| times_in(&harness, job.id, &reason) == 2)
        .await;

    assert_eq!(harness.log.count("download"), 2);
}

#[tokio::test]
async fn another_client_error_fails_with_its_status() {
    let harness = harness_with(DownloadMode::Refuse(vec![400]));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;

    let failed_job = harness
        .repository
        .wait_state(
            job.id,
            &failed(FailureReason::Message(
                "Higgsfield refused to send the result (HTTP 400). Retry to download it again.".into(),
            )),
        )
        .await;

    assert!(failed_job.has_paid_output());
    assert!(!engine.partial_download(job.id).exists());
}

#[tokio::test]
async fn a_download_finishes_in_the_folder_it_started_in() {
    let harness = harness_with(DownloadMode::HangOnce);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_reached(job.id, &JobState::Downloading)
        .await;
    let partial = crate::engine::partial_file(job.id, &harness.output_directory());
    eventually(|| partial.exists()).await;
    let moved = harness.output_directory().join("moved");

    engine.set_output_directory(moved.clone());
    engine.stop(Duration::ZERO).await;
    engine.start().await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.downloader.starts.entries(), ["0", "3"]);
    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(
        output.parent().unwrap().file_name(),
        harness.output_directory().file_name()
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"video");
    assert!(!partial.exists());
    assert!(!moved.exists());
    assert_eq!(done.download_folder, None);
}

// InterleavedWriteTests.

#[tokio::test]
async fn run_next_during_a_submit_keeps_the_submit_marker() {
    let harness = harness();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Hang].into();
    });
    let (reached, release) = harness.repository.hold_save(|job| job.submit_attempt.is_some());
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    reached.wait().await;

    let promoting = tokio::spawn({
        let engine = engine.clone();
        async move { engine.run_next(job.id).await }
    });
    eventually(|| engine.has_write_waiters(job.id)).await;
    release.open();
    promoting.await.unwrap().unwrap();

    let saved = stored(&harness, job.id);
    assert_eq!(saved.submit_attempt, NOT_IDEMPOTENT);
    assert!(saved.promoted_at.is_some());

    eventually(|| submit_keys(&harness).len() == 1).await;
    engine.stop(Duration::ZERO).await;
    let _second = harness.engine().await;
    harness
        .repository
        .wait_reached(job.id, &failed(FailureReason::SubmitCutOff))
        .await;
    assert_eq!(submit_keys(&harness).len(), 1);
}

// StopTests.

#[tokio::test]
async fn start_after_stop_resumes_the_jobs() {
    let harness = harness_with(DownloadMode::HangOnce);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| harness.downloader.starts.entries().len() == 1).await;

    engine.stop(Duration::ZERO).await;
    assert_eq!(engine.runner_count(), 0);
    engine.start().await.unwrap();

    harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
}

/// Quitting while a submit waits for its answer lets the answer arrive, with or without an idempotency key.
#[tokio::test]
async fn stop_lets_a_submit_in_flight_be_answered() {
    for is_idempotent in [true, false] {
        let harness = harness();
        let answer = Gate::new();
        harness.provider.with(|state| {
            state.not_idempotent = !is_idempotent;
            state.submit_steps = [SubmitStep::Answer(answer.clone())].into();
            state.held_status = Some(RemoteStatus::Queued);
        });
        let engine = harness.engine().await;
        let job = enqueue(&engine).await;
        eventually(|| submit_keys(&harness).len() == 1).await;

        let stopping = tokio::spawn({
            let engine = engine.clone();
            async move { engine.stop(Duration::seconds(30)).await }
        });
        eventually(|| engine.submit_waiter_count() == 1).await;
        answer.open();
        stopping.await.unwrap();

        harness.provider.with(|state| state.held_status = None);
        let saved = stored(&harness, job.id);
        assert_eq!(saved.state, JobState::Queued, "{is_idempotent}");
        assert_eq!(saved.submit_attempt, None);
        assert_eq!(saved.remote.unwrap().request_id, "r1");
        let _second = harness.engine().await;
        harness.repository.wait_state(job.id, &JobState::Done).await;
        assert_eq!(submit_keys(&harness), [job.idempotency_key]);
    }
}

#[tokio::test]
async fn stop_sends_no_new_submit_while_it_waits() {
    let harness = harness();
    let answer = Gate::new();
    harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Answer(answer.clone())].into());
    let engine = harness.engine().await;
    let first = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    let stopping = tokio::spawn({
        let engine = engine.clone();
        async move { engine.stop(Duration::seconds(30)).await }
    });
    eventually(|| engine.submit_waiter_count() == 1).await;
    let second = enqueue(&engine).await;
    eventually(|| !engine.has_runner(second.id)).await;
    answer.open();
    stopping.await.unwrap();

    assert_eq!(submit_keys(&harness), [first.idempotency_key]);
    let waiting = stored(&harness, second.id);
    assert_eq!(waiting.state, JobState::WaitingForSlot);
    assert_eq!(waiting.submit_attempt, None);
    let _again = harness.engine().await;
    harness.repository.wait_state(second.id, &JobState::Done).await;
    assert_eq!(
        submit_keys(&harness),
        [first.idempotency_key, second.idempotency_key]
    );
}

// The grace is wall-clock time inside `stop`; Swift's 50 ms is cut to 1 ms so the test barely waits.

#[tokio::test]
async fn stop_cuts_off_an_idempotent_submit_after_its_grace_and_the_next_start_resends_it() {
    let harness = harness();
    harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Hang].into());
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    engine.stop(Duration::milliseconds(1)).await;

    assert_eq!(engine.runner_count(), 0);
    let saved = stored(&harness, job.id);
    assert_eq!(saved.state, JobState::WaitingForSlot);
    assert_eq!(saved.submit_attempt, IDEMPOTENT);
    let _second = harness.engine().await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(submit_keys(&harness), [job.idempotency_key, job.idempotency_key]);
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 1);
    assert_eq!(harness.provider.with(|state| state.uploads.len()), 2);
    assert_eq!(done.submit_attempt, None);
}

#[tokio::test]
async fn stop_cuts_off_a_submit_without_idempotency_after_its_grace_and_the_next_start_fails_it() {
    let harness = harness();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Hang].into();
    });
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    eventually(|| submit_keys(&harness).len() == 1).await;

    engine.stop(Duration::milliseconds(1)).await;

    assert_eq!(stored(&harness, job.id).submit_attempt, NOT_IDEMPOTENT);
    let second = harness.engine().await;
    let failed_job = harness
        .repository
        .wait_state(job.id, &failed(FailureReason::SubmitCutOff))
        .await;
    assert_eq!(failed_job.submit_attempt, NOT_IDEMPOTENT);
    assert_eq!(submit_keys(&harness).len(), 1);
    eventually(|| second.limiter().in_use() == 0).await;
}
