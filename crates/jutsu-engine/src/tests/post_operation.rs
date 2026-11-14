//! Upscale and Remove Background run through the same pipeline as a generation: a port of `PostOperationTests`. A
//! result the account made is named by its job ID instead of uploaded, and the output keeps the server's container and
//! sound, unless the server dropped the sound (Remove Background), which is put back.

use jutsu_core::{
    ClosedRange, DubLanguage, FailureReason, JobSpec, JobState, LocalImage, Mode, PixelSize, PortError,
    PostAction, PostOperation, ProviderError, RemoteInputs, RemoteStatus, Resolution, SourceClip, VideoModel,
};
use url::Url;
use uuid::Uuid;

use super::{AudioMode, Harness, SubmitStep, failed};
use crate::EnqueueOptions;

fn source() -> SourceClip {
    SourceClip::new(
        Url::from_file_path(std::env::temp_dir().join("result.mp4")).unwrap(),
        6.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 6.0),
    )
}

fn spec(action: PostAction, job_id: Option<&str>) -> JobSpec {
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        Mode::MotionTransfer.default_model(),
        Some(source()),
        Resolution::P720,
    );
    spec.operation = Some(PostOperation {
        action,
        source_job_id: job_id.map(Into::into),
    });
    spec
}

fn cutout() -> JobSpec {
    spec(PostAction::RemoveBackground, Some("job-7"))
}

fn with_parent() -> EnqueueOptions {
    EnqueueOptions {
        parent_id: Some(Uuid::new_v4()),
        ..EnqueueOptions::default()
    }
}

fn muxing() -> Harness {
    let mut harness = Harness::new();
    harness.media.audio = AudioMode::Mux;
    harness
}

fn nothing() -> RemoteInputs {
    RemoteInputs {
        video: None,
        images: Vec::new(),
        audio: None,
    }
}

fn submitted(harness: &Harness) -> Vec<RemoteInputs> {
    harness.provider.with(|state| state.submitted_inputs.clone())
}

fn refusal(message: &str) -> SubmitStep {
    SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
        field: None,
        message: message.into(),
    }))
}

fn extension(job: &jutsu_core::Job) -> String {
    let path = job.output_file.as_ref().unwrap().to_file_path().unwrap();
    path.extension().unwrap().to_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_result_named_by_its_job_id_is_neither_prepared_nor_uploaded() {
    let harness = muxing();
    let engine = harness.engine().await;

    let job = engine.enqueue(cutout(), with_parent()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("upload"), 0);
    assert_eq!(submitted(&harness), [nothing()]);
    assert_eq!(
        done.spec.operation.unwrap().source_job_id.as_deref(),
        Some("job-7")
    );
    assert_eq!(done.parent_id, job.parent_id);
}

/// A native draft's finalize names the draft by its job ID (`JobSpec::draft_job_id`): the draft's inputs it keeps
/// for the library are neither prepared nor uploaded.
#[tokio::test]
async fn a_drafts_finalize_uploads_nothing() {
    let harness = muxing();
    let engine = harness.engine().await;
    let mut finalize = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Some(source()),
        Resolution::P1080,
    );
    finalize.references = vec![LocalImage::new(
        Url::from_file_path(std::env::temp_dir().join("a.jpg")).unwrap(),
    )];
    finalize.draft_job_id = Some("draft-job".into());

    let job = engine.enqueue(finalize, with_parent()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("upload"), 0);
    assert_eq!(submitted(&harness), [nothing()]);
    assert_eq!(done.spec.draft_job_id.as_deref(), Some("draft-job"));
}

/// Remove Background's result came without audio in a live run on 2026-10-15, so the source's sound is put back as
/// for a generation, cut to the result's length (`AudioPlan`), and the result is an MP4.
#[tokio::test]
async fn a_cutout_gets_the_sources_sound_back() {
    let harness = muxing();
    let engine = harness.engine().await;

    let job = engine.enqueue(cutout(), with_parent()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("restoreAudio"), 1);
    assert_eq!(extension(&done), "mp4");
}

/// A dub's sound is the point of it: the server's result is kept as it came.
#[tokio::test]
async fn a_dub_keeps_the_servers_sound() {
    let harness = muxing();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(
            spec(PostAction::Dub(DubLanguage::French), Some("job-7")),
            with_parent(),
        )
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("restoreAudio"), 0);
}

#[tokio::test]
async fn without_a_job_id_the_result_file_is_uploaded() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(
            spec(PostAction::RemoveBackground, None),
            EnqueueOptions::default(),
        )
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("upload"), 1);
    assert!(done.inputs.unwrap().video.is_some());
}

#[tokio::test]
async fn a_refused_job_id_is_replaced_by_an_upload_once() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_steps = [refusal("Unknown job_id")].into());
    let engine = harness.engine().await;

    let job = engine.enqueue(cutout(), EnqueueOptions::default()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let submitted = submitted(&harness);
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted.first(), Some(&nothing()));
    assert!(submitted.last().unwrap().video.is_some());
    let operation = done.spec.operation.unwrap();
    assert_eq!(operation.source_job_id, None);
    assert_eq!(operation.action, PostAction::RemoveBackground);
}

#[tokio::test]
async fn a_second_refusal_fails() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_steps = [refusal("Video too long"), refusal("Video too long")].into();
    });
    let engine = harness.engine().await;

    let job = engine.enqueue(cutout(), EnqueueOptions::default()).await.unwrap();

    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Message("Video too long".into())))
        .await;
    assert_eq!(submitted(&harness).len(), 2);
}

#[tokio::test]
async fn the_result_keeps_the_servers_container() {
    let harness = Harness::new();
    let output = Url::parse("https://cdn.example/cutout.MOV?sig=1").unwrap();
    harness
        .provider
        .with(|state| state.status_steps = [Ok(RemoteStatus::Completed { video_url: output })].into());
    let engine = harness.engine().await;

    let job = engine.enqueue(cutout(), EnqueueOptions::default()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(extension(&done), "mov");
}
