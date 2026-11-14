//! Settings › Storage (`StorageSettingsView.swift`, `Storage.swift`): disk use of results, prepared sources, preset
//! previews and downloaded inputs, Clear Cache, and moving old drafts to the trash.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jutsu_core::Job;
use jutsu_store::{FileLayout, StoreError};
use serde::Serialize;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::bytes::{counted, format_bytes};
use crate::environment::AppEnvironment;

/// Disk use shown in Settings › Storage, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageUsage {
    /// Result files of library jobs that are still on disk; other files in the output folder don't count.
    pub results: i64,
    pub result_count: i64,
    pub sources: i64,
    pub cache: i64,
    /// Motion Library clips and account uploads downloaded to be used as inputs.
    pub media: i64,
}

impl StorageUsage {
    /// Measures on the calling thread; call it from `spawn_blocking`. Missing folders and files count as empty.
    pub fn measure(results: &[PathBuf], sources: &Path, cache: &Path, media: &[PathBuf]) -> Self {
        let unique: HashSet<&PathBuf> = results.iter().collect();
        let sizes: Vec<i64> = unique.into_iter().filter_map(|file| file_bytes(file)).collect();
        Self {
            results: sizes.iter().sum(),
            result_count: i64::try_from(sizes.len()).unwrap_or(i64::MAX),
            sources: folder_bytes(&[sources.to_path_buf()]),
            cache: folder_bytes(&[cache.to_path_buf()]),
            media: folder_bytes(media),
        }
    }

    /// Everything Clear Cache deletes.
    pub fn cache_bytes(&self) -> i64 {
        self.sources + self.cache + self.media
    }

    /// "1.43 GB · 12 files", or "Empty".
    pub fn results_text(&self) -> String {
        if self.result_count == 0 {
            return "Empty".to_owned();
        }
        format!(
            "{} · {}",
            format_bytes(self.results),
            counted(self.result_count, "file", "files")
        )
    }
}

/// The size of a regular file; `None` when it is missing or something else.
pub fn file_bytes(path: &Path) -> Option<i64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
}

/// Total size of the regular files under `folders`; missing folders count as empty.
pub fn folder_bytes(folders: &[PathBuf]) -> i64 {
    let mut total = 0;
    let mut pending: Vec<PathBuf> = folders.to_vec();
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total += entry
                    .metadata()
                    .map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX));
            }
        }
    }
    total
}

/// "1.43 GB · prepared sources, previews and downloaded inputs", or "Empty".
pub fn cache_size_text(bytes: i64) -> String {
    if bytes <= 0 {
        return "Empty".to_owned();
    }
    format!(
        "{} · prepared sources, previews and downloaded inputs",
        format_bytes(bytes)
    )
}

fn size_text(bytes: i64) -> String {
    if bytes > 0 {
        format_bytes(bytes)
    } else {
        "Empty".to_owned()
    }
}

/// The Disk use and Cache sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageSummary {
    pub results: String,
    pub sources: String,
    pub cache: String,
    pub media: String,
    /// The speech models' folder; `StorageSummary::new` leaves it empty and `storage_summary` measures it.
    pub speech_models: String,
    pub cache_detail: String,
    /// Jobs still read their prepared files from the cache.
    pub is_busy: bool,
    pub can_clear_cache: bool,
}

impl StorageSummary {
    pub fn new(usage: &StorageUsage, is_busy: bool) -> Self {
        Self {
            results: usage.results_text(),
            sources: size_text(usage.sources),
            cache: size_text(usage.cache),
            media: size_text(usage.media),
            speech_models: size_text(0),
            cache_detail: cache_size_text(usage.cache_bytes()),
            is_busy,
            can_clear_cache: usage.cache_bytes() > 0 && !is_busy,
        }
    }
}

/// A draft result old enough to go to the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldDraft {
    pub job_id: Uuid,
    pub file: PathBuf,
    pub bytes: i64,
}

pub const DRAFT_DAY_OPTIONS: [i32; 3] = [7, 30, 90];
pub const DEFAULT_DRAFT_DAYS: i32 = 30;

/// Finished drafts whose result is still on disk and that finished more than `days` days before `now`.
/// `file_size` gives the size of a file, `None` when it is missing.
pub fn select_old_drafts(
    jobs: &[Job],
    days: i32,
    now: OffsetDateTime,
    file_size: impl Fn(&Path) -> Option<i64>,
) -> Vec<OldDraft> {
    let cutoff = now - Duration::days(i64::from(days));
    jobs.iter()
        .filter(|job| job.spec.is_draft && job.state.is_terminal())
        .filter(|job| job.finished_at.unwrap_or(job.created_at) < cutoff)
        .filter_map(|job| {
            let file = job.output_file.as_ref()?.to_file_path().ok()?;
            let bytes = file_size(&file)?;
            Some(OldDraft {
                job_id: job.id,
                file,
                bytes,
            })
        })
        .collect()
}

fn drafts_count_text(drafts: &[OldDraft]) -> String {
    counted(i64::try_from(drafts.len()).unwrap_or(i64::MAX), "draft", "drafts")
}

fn drafts_size_text(drafts: &[OldDraft]) -> String {
    format_bytes(drafts.iter().map(|draft| draft.bytes).sum())
}

/// "3 drafts · 1.43 GB", or "None".
pub fn old_drafts_summary(drafts: &[OldDraft]) -> String {
    if drafts.is_empty() {
        return "None".to_owned();
    }
    format!("{} · {}", drafts_count_text(drafts), drafts_size_text(drafts))
}

/// The question asked before anything moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub title: String,
    pub message: String,
}

/// `None` when there is nothing to remove.
pub fn old_drafts_confirmation(drafts: &[OldDraft], days: i32) -> Option<Confirmation> {
    if drafts.is_empty() {
        return None;
    }
    let count = drafts_count_text(drafts);
    Some(Confirmation {
        title: format!("Move {count} to the Trash?"),
        message: format!(
            "{count} older than {days} days, {} in total. The library keeps their jobs, inputs and costs and shows the files as missing.",
            drafts_size_text(drafts)
        ),
    })
}

/// The Old drafts section for one choice of days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OldDrafts {
    pub days: i32,
    pub day_options: Vec<i32>,
    pub summary: String,
    pub confirmation: Option<Confirmation>,
    /// The drafts the confirmation counts; exactly these move.
    pub job_ids: Vec<String>,
}

impl OldDrafts {
    pub fn new(drafts: &[OldDraft], days: i32) -> Self {
        Self {
            days,
            day_options: DRAFT_DAY_OPTIONS.to_vec(),
            summary: old_drafts_summary(drafts),
            confirmation: old_drafts_confirmation(drafts, days),
            job_ids: drafts.iter().map(|draft| draft.job_id.to_string()).collect(),
        }
    }
}

/// Why the space couldn't be freed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Can't move {name} to the Trash: {reason}")]
    Trash { name: String, reason: String },
    #[error("Jutsu Studio couldn't finish: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// The jobs of the library's items: what was hidden from the library is not counted.
async fn library_jobs(environment: &AppEnvironment) -> Result<Vec<Job>, StoreError> {
    Ok(environment
        .store
        .materials()
        .await?
        .into_iter()
        .filter(|material| !material.is_hidden)
        .filter_map(|material| material.current.video.map(|job| *job))
        .collect())
}

/// Measures the disk use off the async threads; busy while any job is active.
pub async fn storage_summary(environment: &AppEnvironment) -> Result<StorageSummary, StorageError> {
    let results: Vec<PathBuf> = environment
        .store
        .materials()
        .await?
        .iter()
        .filter(|material| !material.is_hidden)
        .flat_map(|material| {
            material
                .versions()
                .into_iter()
                .filter_map(|version| version.file_url.as_ref()?.to_file_path().ok())
                .collect::<Vec<_>>()
        })
        .collect();
    let is_busy = !environment.store.active_jobs().await?.is_empty();
    let layout = environment.layout.clone();
    let usage = tokio::task::spawn_blocking(move || {
        StorageUsage::measure(
            &results,
            &layout.sources_folder(),
            &layout.cache_folder(),
            &layout.downloaded_input_folders(),
        )
    })
    .await?;
    let mut summary = StorageSummary::new(&usage, is_busy);
    // The speech model the app downloaded, earlier revisions included.
    summary.speech_models = size_text(environment.subtitles.models().size());
    Ok(summary)
}

async fn find_old_drafts(environment: &AppEnvironment, days: i32) -> Result<Vec<OldDraft>, StorageError> {
    let jobs = library_jobs(environment).await?;
    let now = OffsetDateTime::now_utc();
    Ok(tokio::task::spawn_blocking(move || select_old_drafts(&jobs, days, now, file_bytes)).await?)
}

/// The library's drafts older than `days` days.
pub async fn old_drafts(environment: &AppEnvironment, days: i32) -> Result<OldDrafts, StorageError> {
    Ok(OldDrafts::new(&find_old_drafts(environment, days).await?, days))
}

/// Moves the result files of the drafts in `job_ids` that are still older than `days` days to the trash; their
/// records stay, so the library shows the files as missing. Returns the first error after trying every file.
pub async fn trash_old_drafts(
    environment: &AppEnvironment,
    days: i32,
    job_ids: &[Uuid],
) -> Result<(), StorageError> {
    let files: Vec<PathBuf> = find_old_drafts(environment, days)
        .await?
        .into_iter()
        .filter(|draft| job_ids.contains(&draft.job_id))
        .map(|draft| draft.file)
        .collect();
    tokio::task::spawn_blocking(move || trash_files(&files)).await?
}

fn trash_files(files: &[PathBuf]) -> Result<(), StorageError> {
    let mut first_error = None;
    for file in files {
        if let Err(error) = trash::delete(file) {
            tracing::error!(%error, "can't move a draft to the trash");
            first_error.get_or_insert(StorageError::Trash {
                name: file
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                reason: error.to_string(),
            });
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Deletes prepared sources, preset previews and downloaded Motion Library clips and uploads, then recreates the
/// empty folders. The view offers it only while no job is active.
///
/// note: the macOS app keeps the files the Create form points at; Create's draft isn't ported yet, so nothing is
/// kept. Pass them to `FileLayout::clear_cache` once it is.
pub async fn clear_cache(layout: &FileLayout) -> Result<(), StorageError> {
    let layout = layout.clone();
    tokio::task::spawn_blocking(move || {
        layout.clear_cache(&[])?;
        layout.prepare_directories()
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use jutsu_core::{Cost, FailureReason, JobSpec, JobState, Mode, ProviderKind, Resolution};
    use rust_decimal::Decimal;

    use super::*;

    const DAY: i64 = 86_400;

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    fn job(
        draft: bool,
        state: JobState,
        finished_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
        file: Option<&str>,
    ) -> Job {
        let mut spec = JobSpec::new(
            Mode::MotionTransfer,
            Mode::MotionTransfer.default_model(),
            None,
            Resolution::P480,
        );
        spec.is_draft = draft;
        let mut job = Job::new(
            Uuid::new_v4(),
            spec,
            ProviderKind::ApiKey,
            Cost::usd(Decimal::ONE, false),
            Uuid::new_v4(),
            created_at,
        );
        job.state = state;
        job.finished_at = finished_at;
        job.output_file = file.map(crate::testing::file_url);
        job
    }

    fn ago(days: i64) -> OffsetDateTime {
        now() - Duration::seconds(days * DAY)
    }

    fn select(jobs: &[Job], days: i32, missing: &[&str]) -> Vec<OldDraft> {
        select_old_drafts(jobs, days, now(), |path| {
            (!missing.iter().any(|gone| Path::new(gone) == path)).then_some(1_000)
        })
    }

    #[test]
    fn picks_only_old_finished_drafts_with_their_file_on_disk() {
        let old = job(
            true,
            JobState::Done,
            Some(ago(31)),
            now(),
            Some("/results/old.mp4"),
        );
        let jobs = [
            old.clone(),
            job(
                true,
                JobState::Done,
                Some(ago(29)),
                now(),
                Some("/results/recent.mp4"),
            ),
            job(
                false,
                JobState::Done,
                Some(ago(60)),
                now(),
                Some("/results/final.mp4"),
            ),
            job(
                true,
                JobState::Done,
                Some(ago(60)),
                now(),
                Some("/results/gone.mp4"),
            ),
            job(true, JobState::Done, Some(ago(60)), now(), None),
            job(
                true,
                JobState::Processing,
                None,
                ago(60),
                Some("/results/active.mp4"),
            ),
        ];
        let drafts = select(&jobs, 30, &["/results/gone.mp4"]);
        assert_eq!(
            drafts,
            [OldDraft {
                job_id: old.id,
                file: PathBuf::from("/results/old.mp4"),
                bytes: 1_000
            }]
        );
    }

    #[test]
    fn a_job_without_a_finish_date_ages_from_its_creation() {
        let jobs = [job(
            true,
            JobState::Failed {
                reason: FailureReason::Message("x".to_owned()),
            },
            None,
            ago(8),
            Some("/results/a.mp4"),
        )];
        assert_eq!(select(&jobs, 7, &[]).len(), 1);
        assert_eq!(select(&jobs, 30, &[]), []);
    }

    #[test]
    fn confirmation_counts_drafts_and_sums_their_size() {
        let draft = |bytes| OldDraft {
            job_id: Uuid::new_v4(),
            file: PathBuf::from("/a.mp4"),
            bytes,
        };
        let drafts = [draft(1_200_000_000), draft(230_000_000)];
        let confirmation = old_drafts_confirmation(&drafts, 30).unwrap();
        assert_eq!(confirmation.title, "Move 2 drafts to the Trash?");
        assert!(
            confirmation
                .message
                .starts_with("2 drafts older than 30 days, 1.43 GB in total.")
        );
        assert_eq!(
            old_drafts_confirmation(&drafts[..1], 7).unwrap().title,
            "Move 1 draft to the Trash?"
        );
        assert_eq!(old_drafts_confirmation(&[], 30), None);
        assert_eq!(old_drafts_summary(&drafts), "2 drafts · 1.43 GB");
        assert_eq!(old_drafts_summary(&[]), "None");
        let section = OldDrafts::new(&drafts, 30);
        assert_eq!(section.day_options, [7, 30, 90]);
        assert_eq!(section.job_ids.len(), 2);
    }

    #[test]
    fn counts_only_result_files_on_disk_and_each_folder_apart() {
        let root = tempfile::tempdir().unwrap();
        let folder = |name: &str| {
            let path = root.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        let (output, sources, cache) = (folder("output"), folder("sources"), folder("cache"));
        let (clips, uploads) = (folder("motion-library"), folder("account-media"));
        let result = output.join("a.mp4");
        std::fs::write(&result, vec![0; 1_000]).unwrap();
        std::fs::write(output.join("not-in-library.mp4"), vec![0; 5_000]).unwrap();
        std::fs::write(sources.join("s.mp4"), vec![0; 200]).unwrap();
        std::fs::create_dir_all(cache.join("presets")).unwrap();
        std::fs::write(cache.join("presets").join("p.jpg"), vec![0; 30]).unwrap();
        std::fs::write(clips.join("clip.mp4"), vec![0; 400]).unwrap();
        std::fs::write(uploads.join("face.png"), vec![0; 6]).unwrap();

        let usage = StorageUsage::measure(
            &[result.clone(), result, output.join("trashed.mp4")],
            &sources,
            &cache,
            &[clips, uploads, root.path().join("missing")],
        );
        assert_eq!(
            usage,
            StorageUsage {
                results: 1_000,
                result_count: 1,
                sources: 200,
                cache: 30,
                media: 406
            }
        );
        assert_eq!(usage.cache_bytes(), 636);
        assert_eq!(usage.results_text(), "1 KB · 1 file");
        assert_eq!(StorageUsage::default().results_text(), "Empty");
    }

    #[test]
    fn find_reads_real_file_sizes() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("draft.mp4");
        std::fs::write(&file, vec![0; 777]).unwrap();
        let mut draft = job(true, JobState::Done, None, ago(40), None);
        draft.output_file = Some(crate::testing::file_url(&file));
        let drafts = select_old_drafts(&[draft], 30, now(), file_bytes);
        assert_eq!(drafts.iter().map(|draft| draft.bytes).collect::<Vec<_>>(), [777]);
    }

    /// `SettingsTests.cacheSizeText`.
    #[test]
    fn cache_size_text_names_what_it_holds() {
        assert_eq!(cache_size_text(0), "Empty");
        assert_eq!(
            cache_size_text(1_430_000_000),
            "1.43 GB · prepared sources, previews and downloaded inputs"
        );
    }

    #[test]
    fn the_summary_offers_clear_cache_only_with_something_to_clear_and_nothing_running() {
        let usage = StorageUsage {
            sources: 10,
            ..StorageUsage::default()
        };
        assert!(StorageSummary::new(&usage, false).can_clear_cache);
        assert!(!StorageSummary::new(&usage, true).can_clear_cache);
        let empty = StorageSummary::new(&StorageUsage::default(), false);
        assert!(!empty.can_clear_cache);
        assert_eq!(empty.cache_detail, "Empty");
        assert_eq!(empty.sources, "Empty");
        assert_eq!(StorageSummary::new(&usage, false).sources, "10 bytes");
    }

    #[test]
    fn trashing_reports_the_first_failure_after_trying_every_file() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.mp4");
        assert!(matches!(trash_files(&[missing]), Err(StorageError::Trash { .. })));
        assert!(trash_files(&[]).is_ok());
    }

    #[tokio::test]
    async fn clearing_the_cache_keeps_the_folders() {
        let root = tempfile::tempdir().unwrap();
        let layout = FileLayout::new(root.path().join("data"), root.path().join("out"));
        layout.prepare_directories().unwrap();
        std::fs::write(layout.sources_folder().join("s.mp4"), "x").unwrap();
        std::fs::create_dir_all(layout.motion_library_folder()).unwrap();
        std::fs::write(layout.motion_library_folder().join("clip.mp4"), "x").unwrap();
        clear_cache(&layout).await.unwrap();
        assert!(layout.sources_folder().is_dir());
        assert!(!layout.sources_folder().join("s.mp4").exists());
        assert!(!layout.motion_library_folder().exists());
    }
}
