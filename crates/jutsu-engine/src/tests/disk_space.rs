//! A full disk pauses a paid result instead of failing it: a port of `DiskSpaceTests`.
//!
//! Swift's three out-of-space errors (`CocoaError(.fileWriteOutOfSpace)`, `POSIXError(.ENOSPC)`, and a `URLError`
//! carrying `EDQUOT`) reach the Rust engine as `io::ErrorKind::StorageFull` (the first two) and
//! `io::ErrorKind::QuotaExceeded` (the third); there is no `CocoaError` or `URLError` to wrap them in.

use std::io::ErrorKind;

use jutsu_core::{JobState, PauseReason, ProviderError};

use super::{AudioMode, DownloadMode, Harness, eventually, make_spec, paused, quiesce};
use crate::downloader::DownloadError;
use crate::error::{StepError, is_out_of_space};
use crate::{EngineError, EnqueueOptions};

/// The downloads that fail once for want of space, as `DownloadMode::FailOnce` takes them.
const OUT_OF_SPACE: [fn() -> DownloadError; 3] = [
    || DownloadError::Io {
        kind: ErrorKind::StorageFull,
        message: "The file couldn't be saved because there isn't enough space.".into(),
    },
    || DownloadError::Io {
        kind: ErrorKind::StorageFull,
        message: "No space left on device".into(),
    },
    || DownloadError::Io {
        kind: ErrorKind::QuotaExceeded,
        message: "Disc quota exceeded".into(),
    },
];

fn not_enough_space() -> JobState {
    paused(PauseReason::NotEnoughDiskSpace)
}

fn io(kind: ErrorKind) -> StepError {
    StepError::Engine(EngineError::Io {
        kind,
        message: String::new(),
    })
}

#[tokio::test]
async fn a_full_disk_pauses_the_download_and_resume_finishes_it() {
    for error in OUT_OF_SPACE {
        let mut harness = Harness::new();
        harness.downloader.mode = DownloadMode::FailOnce(error);
        let engine = harness.engine().await;

        let job = engine
            .enqueue(make_spec(1), EnqueueOptions::default())
            .await
            .unwrap();
        let held = harness.repository.wait_state(job.id, &not_enough_space()).await;

        assert_eq!(held.remote.as_ref().unwrap().request_id, "r1");
        assert!(held.charged_at.is_some());
        assert!(held.download_folder.is_some());
        let partial = engine.partial_download(job.id);
        assert_eq!(std::fs::read(&partial).unwrap(), b"vid");
        eventually(|| engine.limiter().in_use() == 0).await;

        engine.resume(job.id).await.unwrap();
        let done = harness.repository.wait_state(job.id, &JobState::Done).await;

        assert_eq!(harness.downloader.starts.entries(), ["0", "3"]);
        let output = done.output_file.unwrap().to_file_path().unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"video");
        assert!(!partial.exists());
        assert_eq!(
            harness.provider.with(|state| state.submit_keys.clone()),
            [job.idempotency_key]
        );
    }
}

#[tokio::test]
async fn a_full_disk_while_adding_the_sound_pauses_instead_of_dropping_it() {
    let mut harness = Harness::new();
    harness.media.audio = AudioMode::DiskFullOnce;
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &not_enough_space()).await;

    engine.resume(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    let output = done.output_file.unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(output).unwrap(), b"video+audio");
    assert_eq!(harness.log.count("restoreAudio"), 2);
}

#[tokio::test]
async fn a_job_paused_for_disk_space_stays_paused_after_a_relaunch() {
    let mut harness = Harness::new();
    harness.downloader.mode = DownloadMode::FailOnce(OUT_OF_SPACE[0]);
    let first = harness.engine().await;
    let job = first
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &not_enough_space()).await;
    first.stop(time::Duration::ZERO).await;

    let second = harness.engine().await;
    quiesce().await;

    assert_eq!(
        harness.repository.stored(job.id).unwrap().state,
        not_enough_space()
    );
    assert!(second.partial_download(job.id).exists());
    second.resume(job.id).await.unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;
}

#[test]
fn out_of_space_errors_are_recognized() {
    for kind in [ErrorKind::StorageFull, ErrorKind::QuotaExceeded] {
        assert!(is_out_of_space(kind), "{kind:?}");
        assert!(io(kind).is_out_of_space(), "{kind:?}");
    }
}

#[test]
fn other_write_errors_are_not() {
    // `CocoaError(.fileWriteNoPermission)`, `POSIXError(.ENOENT)`, a bare `URLError(.cannotWriteToFile)` and
    // `ProviderError.network`.
    let errors = [
        io(ErrorKind::PermissionDenied),
        io(ErrorKind::NotFound),
        io(ErrorKind::Other),
        StepError::Provider(ProviderError::Network),
    ];
    for error in errors {
        assert!(!error.is_out_of_space(), "{error:?}");
    }
}
