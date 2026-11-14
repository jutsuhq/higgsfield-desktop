//! Port of `ServerMediaInputsTests`: images of an account element are sent by the media ID the account listed, and
//! uploaded when the provider can't take it. The fake provider is the account `key-a`, as Swift's is by default.

use jutsu_core::{
    ClosedRange, JobSpec, JobState, LocalImage, Mode, PixelSize, PortError, ProviderError, RemoteAsset,
    RemoteInputs, Resolution, ServerMedia, SourceClip,
};
use url::Url;

use super::{Harness, SubmitStep};
use crate::EnqueueOptions;

fn media_url() -> Url {
    Url::parse("https://cdn.example.org/mara.jpg").unwrap()
}

/// A Motion spec over real files whose one reference is an element image of `owner`.
fn spec(folder: &tempfile::TempDir, owner: Option<&str>) -> JobSpec {
    let video = folder.path().join("dance.mov");
    let image = folder.path().join("mara.jpg");
    std::fs::write(&video, b"video").unwrap();
    std::fs::write(&image, b"image").unwrap();
    let source = SourceClip::new(
        Url::from_file_path(&video).unwrap(),
        12.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 12.0),
    );
    let mut reference = LocalImage::new(Url::from_file_path(&image).unwrap());
    reference.server_media = Some(ServerMedia {
        id: "m-1".into(),
        url: media_url(),
        owner: owner.map(str::to_owned),
    });
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        Mode::MotionTransfer.default_model(),
        Some(source),
        Resolution::P480,
    );
    spec.references = vec![reference];
    spec
}

/// A harness whose provider is the account `key-a` and takes the media its account listed.
fn harness() -> Harness {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.owner = Some("key-a".into());
        state.server_media = true;
    });
    harness
}

fn element_image() -> RemoteAsset {
    let mut asset = RemoteAsset::new(media_url());
    asset.id = Some("m-1".into());
    asset
}

#[tokio::test]
async fn an_element_image_is_named_by_its_media_id_and_not_uploaded() {
    let folder = tempfile::tempdir().unwrap();
    let harness = harness();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, Some("key-a")), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("upload"), 1);
    assert_eq!(done.inputs.unwrap().images, [element_image()]);
}

#[tokio::test]
async fn another_accounts_media_is_uploaded() {
    let folder = tempfile::tempdir().unwrap();
    let harness = harness();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, Some("someone-else")), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("upload"), 2);
    assert_ne!(
        done.inputs.unwrap().images.first().unwrap().id.as_deref(),
        Some("m-1")
    );
}

#[tokio::test]
async fn a_refused_media_id_is_uploaded_instead_once() {
    let folder = tempfile::tempdir().unwrap();
    let harness = harness();
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
            field: None,
            message: "Unknown media".into(),
        }))]
        .into();
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(spec(&folder, Some("key-a")), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let submitted = harness.provider.with(|state| state.submitted_inputs.clone());
    assert_eq!(submitted.len(), 2);
    let first_image = |inputs: &RemoteInputs| inputs.images.first().unwrap().id.clone();
    assert_eq!(first_image(&submitted[0]).as_deref(), Some("m-1"));
    assert_ne!(first_image(&submitted[1]).as_deref(), Some("m-1"));
    assert_eq!(done.spec.references.first().unwrap().server_media, None);
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key, job.idempotency_key]
    );
}

#[test]
fn images_stored_before_server_media_still_decode() {
    let image: LocalImage = serde_json::from_str(r#"{"fileURL":"file:///tmp/a.jpg"}"#).unwrap();
    assert_eq!(image.server_media, None);
}
