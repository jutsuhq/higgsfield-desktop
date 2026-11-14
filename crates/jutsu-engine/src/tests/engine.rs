//! The pipeline end to end: a port of `JobEngineTests`.

use std::sync::Arc;

use jutsu_core::{
    AspectRatio, FailureReason, JobSpec, JobState, LocalAudio, LocalImage, Mode, OutputNamePattern,
    PauseReason, PortError, ProviderError, ProviderKind, Rejection, RemoteAsset, RemoteStatus, Resolution,
    UploadCache, VideoModel,
};
use time::Duration;
use url::Url;
use uuid::Uuid;

use super::{
    AudioMode, CallLog, DownloadMode, FakeProvider, Harness, SubmitStep, eventually, failed, make_spec,
    paused, quiesce,
};
use crate::clock::EngineClock;
use crate::engine::EngineEvent;
use crate::{EngineError, EnqueueOptions, JobEngine};

fn key(hash: &str) -> String {
    crate::engine::upload_cache_key(hash, ProviderKind::ApiKey, None)
}

fn provider(error: ProviderError) -> PortError {
    PortError::Provider(error)
}

fn submit_keys(harness: &Harness) -> Vec<Uuid> {
    harness.provider.with(|state| state.submit_keys.clone())
}

fn accepted(harness: &Harness) -> usize {
    harness.provider.with(|state| state.accepted.len())
}

fn hold(harness: &Harness, status: Option<RemoteStatus>) {
    harness.provider.with(|state| state.held_status = status);
}

fn stored_state(harness: &Harness, id: Uuid) -> JobState {
    harness.repository.stored(id).unwrap().state
}

async fn count_in(harness: &Harness, state: &JobState, count: usize) {
    harness
        .repository
        .wait_until(|repository| repository.all().iter().filter(|job| job.state == *state).count() == count)
        .await;
}

async fn enqueue(engine: &JobEngine) -> jutsu_core::Job {
    engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap()
}

fn short_id(id: Uuid) -> String {
    id.simple().to_string()[..8].to_owned()
}

/// A Seedance clip from one image and no source clip, as Swift's `JobSpec(mode:model:source: nil…)`.
fn spec_without_source(prompt: &str) -> JobSpec {
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P720,
    );
    spec.references = vec![LocalImage::new(
        Url::from_file_path(std::env::temp_dir().join("ref0.png")).unwrap(),
    )];
    spec.prompt = prompt.into();
    spec.length = Some(8.0);
    spec.aspect_ratio = Some(AspectRatio::Landscape);
    spec
}

fn file_name(url: &Url) -> String {
    url.to_file_path()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn happy_path_saves_every_state_before_the_next_step() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::Queued), Ok(RemoteStatus::InProgress)].into();
    });
    let engine = harness.engine().await;
    let mut events = engine.subscribe();

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(
        harness.log.entries(),
        [
            "estimate",
            "save preparing",
            "prepareVideo",
            "prepareImage",
            "save uploading",
            "upload",
            "upload",
            "save waitingForSlot",
            "save waitingForSlot",
            "submit",
            "save queued",
            "status",
            "status",
            "save processing",
            "status",
            // The second save records the folder the download goes to.
            "save downloading",
            "save downloading",
            "download",
            "restoreAudio",
            "save done",
        ]
    );
    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
    assert_eq!(done.remote.as_ref().unwrap().request_id, "r1");
    assert!(done.finished_at.is_some());
    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(
        output.parent().unwrap().file_name(),
        harness.output_directory().file_name()
    );
    assert!(output.exists());
    let name = output.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(
        name.ends_with(&format!("_motionTransfer_{}.mp4", short_id(job.id))),
        "{name}"
    );
    let date = name.as_bytes();
    assert!(
        date.len() > 11
            && [0, 1, 2, 3, 5, 6, 8, 9]
                .iter()
                .all(|&at| date[at].is_ascii_digit())
            && date[4] == b'-'
            && date[7] == b'-'
            && date[10] == b'_',
        "{name}"
    );
    loop {
        if let EngineEvent::JobChanged(changed) = events.recv().await.unwrap()
            && changed.id == job.id
            && changed.state == JobState::Done
        {
            break;
        }
    }
}

#[tokio::test]
async fn a_clip_without_a_source_uploads_only_its_images() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(
            spec_without_source("a fox in the snow"),
            EnqueueOptions::default(),
        )
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("prepareImage"), 1);
    assert_eq!(harness.log.count("upload"), 1);
    let submitted = harness
        .provider
        .with(|state| state.submitted_inputs.first().cloned())
        .unwrap();
    assert_eq!(submitted.video, None);
    assert_eq!(submitted.images.len(), 1);
    assert!(done.output_file.is_some());
}

#[tokio::test]
async fn a_speech_is_prepared_and_uploaded_after_the_images() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let mut spec = spec_without_source("two friends talk");
    spec.speech = Some(LocalAudio {
        file_url: Url::from_file_path(std::env::temp_dir().join("speech.mp3")).unwrap(),
        duration: 6.0,
    });

    let job = engine.enqueue(spec, EnqueueOptions::default()).await.unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("prepareAudio"), 1);
    assert_eq!(harness.log.count("upload"), 2);
    let submitted = harness
        .provider
        .with(|state| state.submitted_inputs.first().cloned())
        .unwrap();
    assert_eq!(submitted.images.len(), 1);
    let audio = submitted.audio.unwrap();
    assert_eq!(audio.url.path_segments().unwrap().next_back(), Some("audio"));
}

#[tokio::test]
async fn enqueue_keeps_the_parent() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let parent = Uuid::new_v4();
    let options = EnqueueOptions {
        parent_id: Some(parent),
        ..EnqueueOptions::default()
    };

    let job = engine.enqueue(make_spec(1), options).await.unwrap();

    assert_eq!(job.parent_id, Some(parent));
    assert_eq!(enqueue(&engine).await.parent_id, None);
}

#[tokio::test]
async fn batch_jobs_keep_their_batch_and_share_the_slot_limit() {
    let mut harness = Harness::new();
    harness.max_concurrency = 2;
    hold(&harness, Some(RemoteStatus::Queued));
    let engine = harness.engine().await;
    let batch = Uuid::new_v4();

    let mut jobs = Vec::new();
    for _ in 0..3 {
        let options = EnqueueOptions {
            batch_id: Some(batch),
            ..EnqueueOptions::default()
        };
        jobs.push(engine.enqueue(make_spec(1), options).await.unwrap());
    }
    assert!(jobs.iter().all(|job| job.batch_id == Some(batch)));
    assert_eq!(enqueue(&engine).await.batch_id, None);
    count_in(&harness, &JobState::Queued, 2).await;
    quiesce().await;
    assert_eq!(accepted(&harness), 2);

    hold(&harness, None);
    for job in &jobs {
        harness.repository.wait_reached(job.id, &JobState::Done).await;
    }
    assert_eq!(harness.provider.with(|state| state.max_in_flight), 2);
}

#[tokio::test]
async fn settings_apply_without_restart() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let elsewhere = tempfile::tempdir().unwrap();
    let folder = elsewhere
        .path()
        .join(format!("JobEngineTests-{}", Uuid::new_v4()));
    engine.set_output_directory(folder.clone());
    engine.set_max_concurrency(5);

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(output.parent().unwrap().file_name(), folder.file_name());
    assert!(output.exists());
    assert_eq!(engine.limiter().limit(), 5);
}

#[tokio::test]
async fn name_pattern_applies_to_new_results_without_overwriting() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let first = enqueue(&engine).await;
    let first_done = harness.repository.wait_state(first.id, &JobState::Done).await;

    engine.set_output_name_pattern(OutputNamePattern::new("{mode}"));
    let mut names = Vec::new();
    for _ in 0..2 {
        let job = enqueue(&engine).await;
        let done = harness.repository.wait_state(job.id, &JobState::Done).await;
        names.push(file_name(&done.output_file.unwrap()));
    }

    assert_eq!(names, ["motionTransfer.mp4", "motionTransfer-2.mp4"]);
    let first_output = first_done.output_file.unwrap();
    assert!(file_name(&first_output).ends_with(&format!("_{}.mp4", short_id(first.id))));
    assert!(first_output.to_file_path().unwrap().exists());
}

#[tokio::test]
async fn a_full_queue_requeues_with_the_same_key_after_the_servers_wait() {
    let mut harness = Harness::new();
    harness.max_concurrency = 4;
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(provider(ProviderError::ConcurrencyLimit {
            retry_after: Some(Duration::seconds(42)),
        }))]
        .into();
    });
    let engine = harness.engine().await;
    let start = harness.clock.now();

    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Done).await;

    let waiting = harness
        .repository
        .history(job.id)
        .into_iter()
        .filter(|state| *state == JobState::WaitingForSlot)
        .count();
    // Per submit: the transition in, the attempt marker, and clearing it after the refusal; then the retry.
    assert_eq!(waiting, 6);
    assert_eq!(engine.limiter().limit(), 4);
    assert_eq!(submit_keys(&harness), [job.idempotency_key, job.idempotency_key]);
    assert!(harness.clock.now() - start >= Duration::seconds(42));
}

#[tokio::test]
async fn resume_after_crash_polls_without_resubmitting() {
    let harness = Harness::new();
    hold(&harness, Some(RemoteStatus::Queued));
    let first = harness.engine().await;
    let job = enqueue(&first).await;
    harness.repository.wait_state(job.id, &JobState::Queued).await;
    first.stop(Duration::ZERO).await;

    hold(&harness, None);
    let second = harness.engine().await;
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
    assert_eq!(harness.log.count("prepareVideo"), 1);
    assert_eq!(second.limiter().in_use(), 0);
}

#[tokio::test]
async fn a_failed_load_leaves_the_engine_stopped_so_start_can_run_again() {
    let harness = Harness::new();
    hold(&harness, Some(RemoteStatus::Queued));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Queued).await;
    engine.stop(Duration::ZERO).await;

    hold(&harness, None);
    harness.repository.fail_next_load();
    assert!(engine.start().await.is_err());
    engine.start().await.unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(submit_keys(&harness), [job.idempotency_key]);
}

#[tokio::test]
async fn fresh_upload_cache_entries_skip_the_upload() {
    let harness = Harness::new();
    let cached = RemoteAsset::new(Url::parse("https://upload.example/cached-video").unwrap());
    let now = harness.clock.now();
    harness.cache.store(&cached, &key("video"), now).await.unwrap();
    let stale = RemoteAsset::new(Url::parse("https://upload.example/stale").unwrap());
    harness
        .cache
        .store(&stale, &key("ref0.jpg"), now - Duration::hours(2))
        .await
        .unwrap();
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.provider.with(|state| state.uploads.clone()), ["ref0.jpg"]);
    let inputs = done.inputs.unwrap();
    assert_eq!(inputs.video, Some(cached));
    assert_ne!(inputs.images.first(), Some(&stale));
}

#[tokio::test]
async fn insufficient_funds_pauses_until_resumed() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(provider(ProviderError::InsufficientFunds))].into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let funds = paused(PauseReason::InsufficientFunds);
    harness.repository.wait_state(job.id, &funds).await;
    eventually(|| engine.limiter().in_use() == 0).await;
    assert_eq!(stored_state(&harness, job.id), funds);

    engine.resume(job.id).await.unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(submit_keys(&harness), [job.idempotency_key, job.idempotency_key]);
    assert_eq!(harness.provider.with(|state| state.uploads.len()), 2);
}

#[tokio::test]
async fn unauthorized_before_submit_pauses_until_the_connection_is_back() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.upload_errors = [provider(ProviderError::Unauthorized)].into());
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_state(job.id, &paused(PauseReason::ConnectionNeedsAttention))
        .await;

    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn unauthorized_after_submit_keeps_state_and_stops_polling() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.status_steps = [Err(provider(ProviderError::Unauthorized))].into());
    let engine = harness.engine().await;
    let asked = CallLog::default();
    engine.on_connection_needs_attention({
        let asked = asked.clone();
        move || {
            asked.push("sign in");
            Box::pin(async {})
        }
    });

    let job = enqueue(&engine).await;
    eventually(|| harness.log.count("status") == 1).await;
    quiesce().await;
    assert_eq!(harness.log.count("status"), 1);
    assert_eq!(stored_state(&harness, job.id), JobState::Queued);
    // The job shows no change, so the app is told to ask for the sign-in.
    assert_eq!(asked.count("sign in"), 1);

    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn connection_to_another_provider_pauses_unsubmitted_jobs() {
    let harness = Harness::new();
    harness.connectivity.set(false);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Uploading).await;

    let other = FakeProvider::new(CallLog::default());
    other.with(|state| state.kind = Some(ProviderKind::Account));
    engine.set_provider(Some(Arc::new(other))).await;
    assert_eq!(
        stored_state(&harness, job.id),
        paused(PauseReason::ConnectionNeedsAttention)
    );

    harness.connectivity.set(true);
    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn offline_holds_network_work_until_reconnected() {
    let harness = Harness::new();
    harness.connectivity.set(false);
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Uploading).await;
    quiesce().await;
    assert_eq!(harness.log.count("upload"), 0);
    assert_eq!(stored_state(&harness, job.id), JobState::Uploading);

    harness.connectivity.set(true);
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn validation_error_fails_with_the_message() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(provider(ProviderError::Validation {
            field: Some("prompt".into()),
            message: "Prompt is too long".into(),
        }))]
        .into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_state(
            job.id,
            &failed(FailureReason::Message("Prompt is too long".into())),
        )
        .await;
    eventually(|| engine.limiter().in_use() == 0).await;
}

#[tokio::test]
async fn submit_with_unknown_outcome_fails_without_resending() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(provider(ProviderError::OutcomeUnknown))].into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::OutcomeUnknown))
        .await;
    assert_eq!(harness.log.count("submit"), 1);
    eventually(|| engine.limiter().in_use() == 0).await;
}

#[tokio::test]
async fn remote_rejection_ends_as_rejected() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::Rejected(Rejection::ProtectedContent))].into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let rejected = harness.repository.wait_state(job.id, &JobState::Rejected).await;
    assert_eq!(rejected.rejection, Some(Rejection::ProtectedContent));
    assert_eq!(engine.limiter().in_use(), 0);
}

#[tokio::test]
async fn cancel_before_submit_is_local() {
    let harness = Harness::new();
    harness.connectivity.set(false);
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Uploading).await;

    engine.cancel(job.id).await.unwrap();
    harness.connectivity.set(true);
    quiesce().await;

    let stored = harness.repository.stored(job.id).unwrap();
    assert_eq!(stored.state, JobState::Canceled);
    assert!(stored.charged_at.is_none());
    assert!(harness.provider.with(|state| state.cancels.is_empty()));
    assert_eq!(submit_keys(&harness), [] as [Uuid; 0]);
}

#[tokio::test]
async fn cancel_while_queued_cancels_remotely() {
    let harness = Harness::new();
    hold(&harness, Some(RemoteStatus::Queued));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Queued).await;

    engine.cancel(job.id).await.unwrap();

    assert_eq!(stored_state(&harness, job.id), JobState::Canceled);
    assert_eq!(harness.provider.with(|state| state.cancels.clone()), ["r1"]);
    assert_eq!(engine.limiter().in_use(), 0);
}

#[tokio::test]
async fn cancel_while_processing_is_refused() {
    let harness = Harness::new();
    hold(&harness, Some(RemoteStatus::InProgress));
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Processing).await;

    assert_eq!(
        engine.cancel(job.id).await,
        Err(EngineError::NotCancellable(JobState::Processing))
    );

    hold(&harness, None);
    harness.repository.wait_state(job.id, &JobState::Done).await;
    assert!(harness.provider.with(|state| state.cancels.is_empty()));
}

#[tokio::test]
async fn a_job_canceled_on_the_server_while_queued_is_canceled() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.status_steps = [Ok(RemoteStatus::Canceled)].into());
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;

    harness.repository.wait_state(job.id, &JobState::Canceled).await;
    eventually(|| engine.limiter().in_use() == 0).await;
}

#[tokio::test]
async fn a_job_canceled_on_the_server_while_processing_fails() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::InProgress), Ok(RemoteStatus::Canceled)].into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;

    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::CanceledOnServer))
        .await;
    eventually(|| engine.limiter().in_use() == 0).await;
}

#[tokio::test]
async fn cancel_while_downloading_is_local_and_removes_the_partial_file() {
    let mut harness = Harness::new();
    harness.downloader.mode = DownloadMode::Hang;
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness
        .repository
        .wait_state(job.id, &JobState::Downloading)
        .await;
    let partial = engine.partial_download(job.id);
    eventually(|| partial.exists()).await;

    engine.cancel(job.id).await.unwrap();

    let canceled = harness.repository.stored(job.id).unwrap();
    assert_eq!(canceled.state, JobState::Canceled);
    assert!(canceled.charged_at.is_some());
    assert!(!partial.exists());
    assert!(harness.provider.with(|state| state.cancels.is_empty()));
    assert_eq!(engine.limiter().in_use(), 0);
}

#[tokio::test]
async fn interrupted_download_resumes_from_the_partial_file() {
    let mut harness = Harness::new();
    harness.downloader.mode = DownloadMode::DropOnce;
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.downloader.starts.entries(), ["0", "3"]);
    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"video");
    assert!(!engine.partial_download(job.id).exists());
}

#[tokio::test]
async fn result_with_restored_audio_replaces_the_download() {
    let mut harness = Harness::new();
    harness.media.audio = AudioMode::Mux;
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"video+audio");
    assert!(!engine.partial_download(job.id).exists());
    let leftovers: Vec<_> = std::fs::read_dir(harness.output_directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, [output.file_name().unwrap()]);
}

#[tokio::test]
async fn failed_audio_restore_keeps_the_silent_result() {
    let mut harness = Harness::new();
    harness.media.audio = AudioMode::Fail;
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"video");
    assert_eq!(harness.log.count("restoreAudio"), 1);
}

#[tokio::test]
async fn retry_is_a_new_attempt_with_a_new_idempotency_key() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.status_steps = [Ok(RemoteStatus::Failed {
            message: Some("Generation error".into()),
        })]
        .into();
    });
    let engine = harness.engine().await;

    let job = enqueue(&engine).await;
    let failure = failed(FailureReason::Message("Generation error".into()));
    harness.repository.wait_state(job.id, &failure).await;
    assert_eq!(
        engine.resume(job.id).await,
        Err(EngineError::NotResumable(failure))
    );

    harness.clock.advance(Duration::seconds(600));
    let retried_at = harness.clock.now();
    engine.retry(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_ne!(done.idempotency_key, job.idempotency_key);
    assert_eq!(done.attempt_started_at(), retried_at);
    assert_eq!(submit_keys(&harness), [job.idempotency_key, done.idempotency_key]);
    assert_eq!(accepted(&harness), 2);
}

#[tokio::test]
async fn retry_is_only_for_failed_or_canceled_jobs() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let job = enqueue(&engine).await;
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(
        engine.retry(job.id).await,
        Err(EngineError::NotRetryable(JobState::Done))
    );
    let unknown = Uuid::new_v4();
    assert_eq!(
        engine.cancel(unknown).await,
        Err(EngineError::UnknownJob(unknown))
    );
}
