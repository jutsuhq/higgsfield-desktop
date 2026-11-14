//! Jobs that reuse media already on the provider, and results imported from the account's history: a port of
//! `HistoryTests` (`HistoryEngineTests`).

use jutsu_core::{
    ClosedRange, FailureReason, HistoryItem, Job, JobSpec, JobState, LocalImage, Mode, OutputMedia,
    PixelSize, PortError, ProviderError, RemoteAsset, RemoteInputs, Resolution, SourceClip, VideoModel,
};
use time::{Duration, OffsetDateTime};
use url::Url;

use super::{DownloadMode, Gate, Harness, SubmitStep, eventually, failed};
use crate::clock::EngineClock;
use crate::{EngineError, EnqueueOptions};

fn cdn(path: &str) -> Url {
    Url::parse(&format!("https://cdn.example/{path}")).unwrap()
}

fn asset(path: &str, id: &str) -> RemoteAsset {
    let mut asset = RemoteAsset::new(cdn(path));
    asset.id = Some(id.into());
    asset
}

fn reused() -> RemoteInputs {
    RemoteInputs {
        video: Some(asset("in.mp4", "media-v")),
        images: vec![asset("a.png", "media-a")],
        audio: None,
    }
}

/// A spec whose inputs are already on the provider, as Edit inputs on a history item makes it.
fn reusing_spec() -> JobSpec {
    let inputs = reused();
    let source = SourceClip::new(
        inputs.video.clone().unwrap().url,
        8.0,
        PixelSize::new(1080, 1920),
        ClosedRange::new(0.0, 8.0),
    );
    let mut spec = JobSpec::new(
        Mode::ObjectSwap,
        Mode::ObjectSwap.default_model(),
        Some(source),
        Resolution::P720,
    );
    spec.references = inputs
        .images
        .iter()
        .map(|image| LocalImage::new(image.url.clone()))
        .collect();
    spec.remote_inputs = Some(inputs);
    spec
}

fn history_item() -> HistoryItem {
    let mut item = HistoryItem::new(
        "remote-1",
        Mode::ObjectSwap,
        VideoModel::GENJUTSU,
        OffsetDateTime::from_unix_timestamp(1_791_032_606).unwrap(),
        cdn("out.mp4"),
        reused(),
    );
    item.duration = Some(8.0);
    item
}

fn importing(harness: &Harness, item: &HistoryItem) -> Job {
    Job::importing(item, Some("user_1".into()), harness.clock.now())
}

fn harness_with(download: DownloadMode) -> Harness {
    let mut harness = Harness::new();
    harness.downloader.mode = download;
    harness
}

fn media_not_found() -> SubmitStep {
    SubmitStep::Fail(PortError::Provider(ProviderError::Validation {
        field: None,
        message: "Media not found".into(),
    }))
}

fn throttled() -> EngineError {
    EngineError::Port(PortError::Provider(ProviderError::Server { status: 429 }))
}

#[tokio::test]
async fn reused_media_ids_are_sent_without_preparing_or_uploading() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(reusing_spec(), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(
        harness.provider.with(|state| state.submitted_inputs.clone()),
        [reused()]
    );
    assert_eq!(harness.log.count("prepareVideo"), 0);
    assert_eq!(harness.log.count("upload"), 0);
    assert_eq!(done.inputs, Some(reused()));
    assert_eq!(done.spec.remote_inputs, Some(reused()));
}

#[tokio::test]
async fn refused_media_ids_are_downloaded_and_uploaded_again_once() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_steps = [media_not_found()].into());
    let engine = harness.engine().await;

    let job = engine
        .enqueue(reusing_spec(), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let submitted = harness.provider.with(|state| state.submitted_inputs.clone());
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted.first(), Some(&reused()));
    assert_eq!(
        submitted.last().unwrap().video.as_ref().unwrap().url.host_str(),
        Some("upload.example")
    );
    assert_eq!(done.spec.remote_inputs, None);
    let after_refusal: Vec<String> = harness
        .log
        .entries()
        .into_iter()
        .skip_while(|entry| entry != "submit")
        .skip(1)
        .filter(|entry| ["download", "prepareVideo", "prepareImage", "upload"].contains(&entry.as_str()))
        .take(6)
        .collect();
    assert_eq!(
        after_refusal,
        [
            "download",
            "prepareVideo",
            "download",
            "prepareImage",
            "upload",
            "upload"
        ]
    );
    let prefix = format!("jutsu-{}", jutsu_core::support::uuid_string(job.id));
    let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .collect();
    assert!(leftovers.is_empty());
}

#[tokio::test]
async fn a_second_refusal_fails_the_job() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_steps = [media_not_found(), media_not_found()].into());
    let engine = harness.engine().await;

    let job = engine
        .enqueue(reusing_spec(), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::Message("Media not found".into())))
        .await;

    assert_eq!(harness.provider.with(|state| state.submitted_inputs.len()), 2);
}

#[tokio::test]
async fn import_downloads_the_result_once_and_saves_a_finished_job() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);

    let imported = engine
        .import_finished(job.clone(), item.output_url.clone())
        .await
        .unwrap();
    let again = engine
        .import_finished(job.clone(), item.output_url.clone())
        .await
        .unwrap();

    let output = imported.output_file.clone().unwrap().to_file_path().unwrap();
    assert_eq!(
        output.parent().unwrap().file_name(),
        harness.output_directory().file_name()
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"video");
    assert!(imported.state == JobState::Done && imported.remote.as_ref().unwrap().request_id == "remote-1");
    assert_eq!(harness.repository.stored(job.id), Some(imported.clone()));
    assert_eq!(again, imported);
    assert_eq!(harness.log.count("download"), 1);
    assert!(!engine.partial_download(job.id).exists());
}

#[tokio::test]
async fn expired_result_is_not_imported() {
    let harness = harness_with(DownloadMode::Refuse(vec![404]));
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);

    assert_eq!(
        engine.import_finished(job.clone(), item.output_url.clone()).await,
        Err(EngineError::Port(PortError::Failure(
            FailureReason::OutputUnavailable
        )))
    );
    assert_eq!(harness.repository.stored(job.id), None);
}

#[tokio::test]
async fn a_throttled_import_can_be_tried_again() {
    let harness = harness_with(DownloadMode::Refuse(vec![429]));
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);

    assert_eq!(
        engine.import_finished(job.clone(), item.output_url.clone()).await,
        Err(throttled())
    );
    let imported = engine
        .import_finished(job, item.output_url.clone())
        .await
        .unwrap();

    assert!(imported.output_file.is_some());
}

#[tokio::test]
async fn a_running_import_keeps_its_partial_file_through_a_restart() {
    let harness = harness_with(DownloadMode::Hang);
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);
    let importing = tokio::spawn({
        let engine = engine.clone();
        let (job, url) = (job.clone(), item.output_url.clone());
        async move { engine.import_finished(job, url).await }
    });
    let partial = engine.partial_download(job.id);
    eventually(|| partial.exists()).await;

    engine.stop(Duration::ZERO).await;
    engine.start().await.unwrap();

    assert!(partial.exists());
    importing.abort();
    let _ = importing.await;
}

/// A second import of a job whose import is running (an agent's `still_working` retry) waits for that one: it never
/// starts a second download of the same result.
#[tokio::test]
async fn a_second_import_of_the_same_job_waits_for_the_running_one() {
    let release = Gate::new();
    let harness = harness_with(DownloadMode::HoldOnce(release.clone()));
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);
    let import = |engine: crate::JobEngine| {
        let (job, url) = (job.clone(), item.output_url.clone());
        async move { engine.import_finished(job, url).await }
    };
    let first = tokio::spawn(import(engine.clone()));
    eventually(|| harness.log.count("download") == 1).await;

    // Sent while the first downloads; on its own it would download at once.
    let started = Gate::new();
    let second = tokio::spawn({
        let (started, import) = (started.clone(), import(engine.clone()));
        async move {
            started.open();
            import.await
        }
    });
    started.wait().await;
    release.open();

    let imported = first.await.unwrap().unwrap();
    assert_eq!(second.await.unwrap().unwrap().id, imported.id);
    assert!(imported.output_file.is_some());
    assert_eq!(harness.log.count("download"), 1);
}

#[tokio::test]
async fn an_abandoned_import_partial_is_deleted_on_the_next_start() {
    let harness = harness_with(DownloadMode::Refuse(vec![429]));
    let engine = harness.engine().await;
    let item = history_item();
    let job = importing(&harness, &item);
    let partial = engine.partial_download(job.id);
    assert_eq!(
        engine.import_finished(job, item.output_url.clone()).await,
        Err(throttled())
    );
    std::fs::write(&partial, b"vid").unwrap();

    let _engine = harness.engine().await;

    assert!(!partial.exists());
}

/// Issue #134: `show_generations` params said 854×480 and 5 s of a 480×854, 3.71 s Edit; the file decides.
#[tokio::test]
async fn an_import_keeps_the_files_own_length_and_size_over_the_params() {
    let file = OutputMedia {
        duration: 3.71,
        pixel_size: PixelSize::new(480, 854),
    };
    let mut harness = Harness::new();
    harness.media.output_media = Some(file);
    let engine = harness.engine().await;
    let mut item = history_item();
    item.duration = Some(5.0);
    item.pixel_size = Some(PixelSize::new(854, 480));
    let job = importing(&harness, &item);
    assert!((job.output_duration() - 5.0).abs() < f64::EPSILON);

    let imported = engine
        .import_finished(job.clone(), item.output_url.clone())
        .await
        .unwrap();

    assert_eq!(imported.output_media, Some(file));
    assert!((imported.output_duration() - 3.71).abs() < f64::EPSILON);
    assert_eq!(imported.output_pixel_size(), Some(file.pixel_size));
    assert_eq!(
        harness.repository.stored(job.id).unwrap().output_media,
        Some(file)
    );
}

#[tokio::test]
async fn a_generated_job_keeps_its_files_own_length() {
    let file = OutputMedia {
        duration: 7.5,
        pixel_size: PixelSize::new(720, 1280),
    };
    let mut harness = Harness::new();
    harness.media.output_media = Some(file);
    let engine = harness.engine().await;

    let job = engine
        .enqueue(reusing_spec(), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.output_media, Some(file));
    assert!((done.output_duration() - 7.5).abs() < f64::EPSILON);
}

#[tokio::test]
async fn a_result_that_cant_be_read_falls_back_to_the_spec() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(reusing_spec(), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.output_media, None);
    assert!((done.output_duration() - 8.0).abs() < f64::EPSILON);
    assert_eq!(done.output_pixel_size(), Some(PixelSize::new(1080, 1920)));
}
