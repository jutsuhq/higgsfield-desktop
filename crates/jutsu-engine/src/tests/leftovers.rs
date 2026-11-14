//! Port of `LeftoversTests` (`ScratchFileTests` and `LeftoverCleanupTests`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jutsu_core::support::uuid_string;
use jutsu_core::{Job, JobRepository, JobState, PauseReason, ProviderKind, RemoteAsset, UploadCache};
use time::Duration;
use url::Url;
use uuid::Uuid;

use super::{Harness, make_spec, usd};
use crate::clock::EngineClock;
use crate::{JobEngine, ScratchFile};

// ScratchFile.

#[test]
fn only_scratch_names_of_inactive_jobs_are_leftovers() {
    let (kept, gone) = (uuid_string(Uuid::new_v4()), uuid_string(Uuid::new_v4()));
    let list: Vec<String> = [
        format!(".{kept}.partial"),
        format!(".{kept}.audio.mp4"),
        format!(".{gone}.partial"),
        format!(".{gone}.audio.mp4"),
        "2026-03-07_restyle_1a2b3c4d.mp4".to_owned(),
        format!("{gone}.partial"),
        format!(".{gone}.partial.mp4"),
        format!(".{gone}.mov"),
        ".not-a-uuid.partial".to_owned(),
        ".DS_Store".to_owned(),
        ".".to_owned(),
        String::new(),
    ]
    .into();
    let keeping: HashSet<Uuid> = [Uuid::parse_str(&kept).unwrap()].into();
    assert_eq!(
        ScratchFile::leftovers(&list, &keeping, true),
        [format!(".{gone}.partial"), format!(".{gone}.audio.mp4")]
    );
}

#[test]
fn unowned_audio_scratch_stays_while_a_job_runs() {
    let list = vec![
        format!(".{}.mp4", uuid_string(Uuid::new_v4())),
        format!(".{}.m4a", uuid_string(Uuid::new_v4())),
    ];
    assert_eq!(
        ScratchFile::leftovers(&list, &HashSet::new(), true),
        Vec::<String>::new()
    );
    assert_eq!(ScratchFile::leftovers(&list, &HashSet::new(), false), list);
}

#[test]
fn engine_names_parse_as_their_job() {
    let kept = Uuid::new_v4();
    let folder = Path::new("output");
    let name = |file: PathBuf| file.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(
        ScratchFile::parse(&name(crate::engine::partial_file(kept, folder))),
        Some(ScratchFile::Job(kept))
    );
    assert_eq!(
        ScratchFile::parse(&name(JobEngine::audio_file(kept, folder))),
        Some(ScratchFile::Job(kept))
    );
}

/// Enhance on this computer writes `.ondevice-<UUID>.partial` and clears its own; the sweep leaves a running one be.
#[test]
fn on_device_partials_are_not_a_jobs() {
    let name = format!(".ondevice-{}.partial", uuid_string(Uuid::new_v4()));
    assert_eq!(ScratchFile::parse(&name), None);
    assert_eq!(
        ScratchFile::leftovers(&[name], &HashSet::new(), false),
        Vec::<String>::new()
    );
}

// Cleanup at start.

fn write(name: &str, folder: &Path) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let file = folder.join(name);
    std::fs::write(&file, b"vid").unwrap();
    file
}

/// A job as the store keeps it, of `owner`, in `state`.
fn stored_job(harness: &Harness, owner: &str, state: JobState) -> Job {
    let mut job = Job::new(
        Uuid::new_v4(),
        make_spec(1),
        ProviderKind::ApiKey,
        usd("1.30"),
        Uuid::new_v4(),
        harness.clock.now(),
    );
    job.owner = Some(owner.to_owned());
    job.state = state;
    job
}

#[tokio::test]
async fn start_deletes_only_scratch_files_no_active_job_owns() {
    let harness = Harness::new();
    let output = harness.output_directory();
    let other_folder = output.join("earlier-folder");
    let mut paused = stored_job(&harness, "key-a", JobState::Paused(PauseReason::ByUser));
    paused.download_folder = Some(Url::from_file_path(&other_folder).unwrap());
    let mut finished = stored_job(&harness, "key-a", JobState::Done);
    let result = write("2026-03-07_motionTransfer_1a2b3c4d.mp4", &output);
    finished.output_file = Some(Url::from_file_path(&result).unwrap());
    harness.repository.save(&paused).await.unwrap();
    harness.repository.save(&finished).await.unwrap();

    let paused_id = uuid_string(paused.id);
    let finished_id = uuid_string(finished.id);
    let paused_partial = write(&format!(".{paused_id}.partial"), &other_folder);
    let paused_audio = write(&format!(".{paused_id}.audio.mp4"), &other_folder);
    let user_files = [
        write(".hidden-by-the-user.mp4", &output),
        write("notes.partial", &output),
        write(&format!("{finished_id}.partial"), &output),
    ];
    let leftovers = [
        write(&format!(".{finished_id}.partial"), &output),
        write(&format!(".{}.partial", uuid_string(Uuid::new_v4())), &output),
        write(
            &format!(".{}.audio.mp4", uuid_string(Uuid::new_v4())),
            &other_folder,
        ),
        write(&format!(".{}.mp4", uuid_string(Uuid::new_v4())), &output),
        write(&format!(".{}.m4a", uuid_string(Uuid::new_v4())), &other_folder),
    ];
    let scratch_named_folder = output.join(format!(".{}.partial", uuid_string(Uuid::new_v4())));
    std::fs::create_dir_all(&scratch_named_folder).unwrap();

    let _engine = harness.engine().await;

    assert!(leftovers.iter().all(|file| !file.exists()));
    assert!(result.exists());
    assert!(paused_partial.exists() && paused_audio.exists());
    assert!(user_files.iter().all(|file| file.exists()));
    assert!(scratch_named_folder.exists());
}

#[tokio::test]
async fn start_keeps_scratch_files_of_a_record_that_did_not_decode() {
    let harness = Harness::new();
    let unreadable = Uuid::new_v4();
    harness.repository.undecodable.lock().unwrap().insert(unreadable);
    let output = harness.output_directory();
    let partial = write(&format!(".{}.partial", uuid_string(unreadable)), &output);
    let audio = write(&format!(".{}.audio.mp4", uuid_string(unreadable)), &output);
    let abandoned = write(&format!(".{}.partial", uuid_string(Uuid::new_v4())), &output);

    let _engine = harness.engine().await;

    assert!(partial.exists() && audio.exists());
    assert!(!abandoned.exists());
}

#[tokio::test]
async fn start_forgets_uploads_too_old_to_reuse() {
    let harness = Harness::new();
    let asset = RemoteAsset::new(Url::parse("https://upload.example/a").unwrap());
    let now = harness.clock.now();
    harness
        .cache
        .store(&asset, "fresh", now - Duration::seconds(3600))
        .await
        .unwrap();
    harness
        .cache
        .store(&asset, "stale", now - Duration::seconds(3601))
        .await
        .unwrap();

    let _engine = harness.engine().await;

    let hashes: Vec<String> = harness.cache.0.lock().unwrap().keys().cloned().collect();
    assert_eq!(hashes, ["fresh"]);
}
