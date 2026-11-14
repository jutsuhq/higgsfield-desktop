//! Port of `LinkedInputsTests`: inputs downloaded from a link are sent by that link where the provider takes it,
//! uploaded otherwise.

use jutsu_core::{
    ClosedRange, FailureReason, JobSpec, JobState, LocalImage, Mode, PixelSize, PortError, ProviderError,
    RemoteAsset, RemoteInputs, Resolution, SourceClip,
};
use url::Url;

use super::{Harness, SubmitStep, failed};
use crate::EnqueueOptions;

fn video_origin() -> Url {
    Url::parse("https://cdn.example.org/dance.mp4").unwrap()
}

fn image_origin() -> Url {
    Url::parse("https://cdn.example.org/face.png").unwrap()
}

/// A spec over real files, so their size can be read; the clip is 12 s long.
fn spec(folder: &tempfile::TempDir, range: (f64, f64), linked: bool) -> JobSpec {
    let video = folder.path().join("dance.mp4");
    let image = folder.path().join("face.png");
    std::fs::write(&video, b"video").unwrap();
    std::fs::write(&image, b"image").unwrap();
    let mut source = SourceClip::new(
        Url::from_file_path(&video).unwrap(),
        12.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(range.0, range.1),
    );
    source.remote_origin = linked.then(video_origin);
    let mut reference = LocalImage::new(Url::from_file_path(&image).unwrap());
    reference.remote_origin = linked.then(image_origin);
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        Mode::MotionTransfer.default_model(),
        Some(source),
        Resolution::P480,
    );
    spec.references = vec![reference];
    spec
}

fn whole(folder: &tempfile::TempDir) -> JobSpec {
    spec(folder, (0.0, 12.0), true)
}

fn validation(field: Option<&str>, message: &str) -> SubmitStep {
    SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
        field: field.map(str::to_owned),
        message: message.to_owned(),
    }))
}

#[tokio::test]
async fn a_whole_linked_clip_makes_no_upload() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness.provider.with(|state| state.linked = true);
    let engine = harness.engine().await;

    let job = engine
        .enqueue(whole(&folder), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("upload"), 0);
    assert_eq!(
        harness.provider.with(|state| state.submitted_inputs.clone()),
        done.inputs.iter().cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        done.inputs,
        Some(RemoteInputs {
            video: Some(RemoteAsset::new(video_origin())),
            images: vec![RemoteAsset::new(image_origin())],
            audio: None,
        })
    );
}

#[tokio::test]
async fn a_trimmed_clip_is_uploaded_and_its_linked_image_is_not() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness.provider.with(|state| state.linked = true);
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, (2.0, 10.0), true), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("upload"), 1);
    assert_eq!(harness.log.count("linked video"), 0);
    let inputs = done.inputs.unwrap();
    assert_ne!(inputs.video.unwrap().url, video_origin());
    assert_eq!(inputs.images, [RemoteAsset::new(image_origin())]);
}

#[tokio::test]
async fn a_provider_that_doesnt_take_links_gets_uploads() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(whole(&folder), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("linked video"), 1);
    assert_eq!(harness.log.count("upload"), 2);
    assert_ne!(done.inputs.unwrap().video.unwrap().url, video_origin());
}

#[tokio::test]
async fn picked_files_are_never_offered_by_link() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness.provider.with(|state| state.linked = true);
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, (0.0, 12.0), false), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert!(
        !harness
            .log
            .entries()
            .iter()
            .any(|entry| entry.starts_with("linked"))
    );
    assert_eq!(harness.log.count("upload"), 2);
}

#[tokio::test]
async fn a_refused_link_is_uploaded_instead() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.linked = true;
        state.submit_steps = [validation(Some("video_url"), "Can't fetch")].into();
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(whole(&folder), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let submitted = harness.provider.with(|state| state.submitted_inputs.clone());
    assert_eq!(submitted.len(), 2);
    assert_eq!(
        submitted.first().unwrap().video.as_ref().unwrap().url,
        video_origin()
    );
    assert_ne!(
        submitted.last().unwrap().video.as_ref().unwrap().url,
        video_origin()
    );
    assert_eq!(harness.log.count("upload"), 2);
    assert_eq!(done.spec.source.unwrap().remote_origin, None);
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key, job.idempotency_key]
    );
}

#[tokio::test]
async fn other_refusals_of_uploaded_inputs_still_fail() {
    let folder = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_steps = [validation(Some("prompt"), "Too long")].into());
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, (0.0, 12.0), false), EnqueueOptions::default())
        .await
        .unwrap();

    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Message("Too long".into())))
        .await;
    assert_eq!(harness.provider.with(|state| state.submitted_inputs.len()), 1);
}
