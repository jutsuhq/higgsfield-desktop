//! Port of `ServerURLTests` (`ServerURLEngineTests`): reused inputs that name local files, and the generation
//! deadline.

use std::path::PathBuf;

use jutsu_core::{
    ClosedRange, FailureReason, Job, JobRepository, JobSpec, JobState, LocalImage, Mode, PixelSize,
    PortError, ProviderError, ProviderKind, RemoteAsset, RemoteInputs, RemoteStatus, Resolution, SourceClip,
};
use time::Duration;
use url::Url;
use uuid::Uuid;

use super::{Harness, SubmitStep, eventually, failed, make_spec, usd};
use crate::clock::EngineClock;
use crate::{EngineError, EngineSettings, EnqueueOptions, JobEngine};

fn deadline() -> Duration {
    EngineSettings::new(PathBuf::from("output")).generation_deadline
}

/// Reused inputs as a tampered history entry would give them: the source video names a local file.
fn spec_reusing_a_local_file() -> JobSpec {
    let local = Url::parse("file:///home/me/Movies/private.mov").unwrap();
    let mut image = RemoteAsset::new(Url::parse("https://cdn.example/a.png").unwrap());
    image.id = Some("media-a".into());
    let mut video = RemoteAsset::new(local.clone());
    video.id = Some("media-v".into());
    let source = SourceClip::new(local, 8.0, PixelSize::new(1080, 1920), ClosedRange::new(0.0, 8.0));
    let mut spec = JobSpec::new(
        Mode::ObjectSwap,
        Mode::ObjectSwap.default_model(),
        Some(source),
        Resolution::P720,
    );
    spec.references = vec![LocalImage::new(image.url.clone())];
    spec.remote_inputs = Some(RemoteInputs {
        video: Some(video),
        images: vec![image],
        audio: None,
    });
    spec
}

fn deadline_failure() -> JobState {
    failed(FailureReason::Deadline)
}

#[tokio::test]
async fn reused_inputs_that_are_local_files_fail_the_job() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec_reusing_a_local_file(), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::InsecureInput))
        .await;

    assert!(harness.provider.with(|state| state.submitted_inputs.is_empty()));
    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("download"), 0);
}

#[tokio::test]
async fn refused_reused_inputs_are_never_read_as_local_files() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.owner = Some("key-a".into());
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "Media not found".into(),
        }))]
        .into();
    });
    let mut stored = Job::new(
        Uuid::new_v4(),
        spec_reusing_a_local_file(),
        ProviderKind::ApiKey,
        usd("1.30"),
        Uuid::new_v4(),
        harness.clock.now(),
    );
    stored.owner = Some("key-a".into());
    stored.state = JobState::WaitingForSlot;
    harness.repository.save(&stored).await.unwrap();

    let _engine = harness.engine().await;
    let failed = harness
        .repository
        .wait_state(stored.id, &failed(FailureReason::InsecureInput))
        .await;

    assert!(failed.spec.remote_inputs.is_some());
    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("download"), 0);
    assert_eq!(harness.log.count("upload"), 0);
}

#[tokio::test]
async fn a_job_the_provider_never_finishes_fails_at_the_deadline() {
    let mut harness = Harness::new();
    harness.max_concurrency = 1;
    harness.generation_deadline = deadline();
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let failed = harness.repository.wait_state(job.id, &deadline_failure()).await;

    let submitted_at = failed.submitted_at.unwrap();
    assert!(harness.clock.now() - submitted_at >= deadline());
    assert_eq!(harness.provider.with(|state| state.submit_keys.len()), 1);
    assert_eq!(engine.limiter().in_use(), 0);

    harness.provider.with(|state| state.held_status = None);
    let next = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(next.id, &JobState::Done).await;
    assert_eq!(
        harness.repository.stored(job.id).unwrap().state,
        deadline_failure()
    );
    assert_eq!(harness.provider.with(|state| state.submit_keys.len()), 2);
}

#[tokio::test]
async fn a_job_still_queued_at_the_deadline_fails_after_polling_through_it() {
    let mut harness = Harness::new();
    harness.generation_deadline = deadline();
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::Queued));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &deadline_failure()).await;

    assert!(harness.log.count("status") > 100);
}

/// Fails a job at the deadline while the provider keeps generating it.
async fn time_out(harness: &Harness) -> (JobEngine, Job) {
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let failed = harness.repository.wait_state(job.id, &deadline_failure()).await;
    assert!(failed.may_still_finish());
    (engine, failed)
}

fn deadline_harness() -> Harness {
    let mut harness = Harness::new();
    harness.generation_deadline = deadline();
    harness
}

#[tokio::test]
async fn check_again_after_the_deadline_fetches_the_result_without_submitting_again() {
    let harness = deadline_harness();
    let (engine, failed) = time_out(&harness).await;

    harness.provider.with(|state| state.held_status = None);
    engine.check_again(failed.id).await.unwrap();
    let done = harness.repository.wait_state(failed.id, &JobState::Done).await;

    assert_eq!(harness.provider.with(|state| state.submit_keys.len()), 1);
    assert_eq!(done.remote, failed.remote);
    assert_eq!(done.timed_out_at, None);
}

#[tokio::test]
async fn retry_after_the_deadline_checks_again_instead_of_submitting() {
    let harness = deadline_harness();
    let (engine, failed) = time_out(&harness).await;

    harness.provider.with(|state| state.held_status = None);
    engine.retry(failed.id).await.unwrap();
    let done = harness.repository.wait_state(failed.id, &JobState::Done).await;

    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [failed.idempotency_key]
    );
    assert_eq!(done.idempotency_key, failed.idempotency_key);
}

#[tokio::test]
async fn check_again_starts_a_new_deadline() {
    let harness = deadline_harness();
    let (engine, failed) = time_out(&harness).await;

    engine.check_again(failed.id).await.unwrap();
    eventually(|| {
        harness
            .repository
            .history(failed.id)
            .iter()
            .filter(|state| **state == failed.state)
            .count()
            == 2
    })
    .await;

    let again = harness.repository.stored(failed.id).unwrap();
    let checked_again_at = again.checked_again_at.unwrap();
    let first_failure = failed.finished_at.unwrap();
    assert!(checked_again_at >= first_failure);
    assert!(harness.clock.now() - checked_again_at >= deadline());
    assert!(again.may_still_finish());
    assert_eq!(harness.provider.with(|state| state.submit_keys.len()), 1);
}

#[tokio::test]
async fn only_a_job_past_its_deadline_is_checked_again() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::Failed {
            message: Some("Server error".into()),
        })]
        .into();
    });
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let message = failed(FailureReason::Message("Server error".into()));
    harness.repository.wait_state(job.id, &message).await;

    assert_eq!(
        engine.check_again(job.id).await,
        Err(EngineError::NotRetryable(message))
    );
}
