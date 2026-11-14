//! What crashed or abandoned downloads leave behind.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jutsu_core::Job;
use jutsu_core::support::uuid_string;
use uuid::Uuid;

use super::JobEngine;

impl JobEngine {
    /// Deletes what crashed or abandoned downloads left behind, and upload-cache entries too old to be reused. Only
    /// hidden scratch files (`ScratchFile`) in the output folder and in the folders active jobs download to are
    /// looked at; those of active jobs and running imports stay, and so does every other file. `stored_ids` is every
    /// active job in the repository, including ones that didn't decode.
    pub(crate) async fn remove_leftovers(&self, stored: &[Job], stored_ids: &HashSet<Uuid>) {
        let settings = self.settings();
        let (folders, kept, is_any_job_running) = {
            let state = self.st();
            let active: Vec<&Job> = stored
                .iter()
                .chain(state.jobs.values().filter(|job| job.state.is_active()))
                .collect();
            let mut kept: HashSet<Uuid> = active.iter().map(|job| job.id).collect();
            kept.extend(stored_ids);
            kept.extend(state.runners.keys());
            kept.extend(state.importing.keys());
            let mut folders: Vec<PathBuf> = vec![settings.output_directory.clone()];
            for folder in active
                .iter()
                .filter_map(|job| job.download_folder.as_ref())
                .filter_map(|folder| folder.to_file_path().ok())
            {
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
            (folders, kept, !state.runners.is_empty())
        };
        for folder in folders {
            for file in leftovers(&folder, &kept, is_any_job_running) {
                if let Err(error) = std::fs::remove_file(&file) {
                    tracing::error!(%error, "can't delete a leftover scratch file");
                }
            }
        }
        let cutoff = self.now() - settings.upload_max_age;
        if let Err(error) = self.0.upload_cache.remove_assets(cutoff).await {
            tracing::error!(%error, "can't forget old uploads");
        }
    }

    /// The result with its sound restored, until it is placed.
    pub(crate) fn audio_file(id: Uuid, folder: &Path) -> PathBuf {
        folder.join(format!(".{}.audio.mp4", uuid_string(id)))
    }
}

/// Kept next to the result so the final rename stays on one volume, and across retries and relaunches so the
/// download resumes.
pub(crate) fn partial_file(id: Uuid, folder: &Path) -> PathBuf {
    folder.join(format!(".{}.partial", uuid_string(id)))
}

/// The regular files in `folder` that `ScratchFile::leftovers` picks; none when it can't be read.
pub(crate) fn leftovers(folder: &Path, kept: &HashSet<Uuid>, is_any_job_running: bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let regular: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect();
    let names: Vec<String> = regular
        .iter()
        .filter_map(|file| file.file_name().and_then(|name| name.to_str()).map(str::to_owned))
        .collect();
    let picked: HashSet<String> = ScratchFile::leftovers(&names, kept, is_any_job_running)
        .into_iter()
        .collect();
    regular
        .into_iter()
        .filter(|file| {
            file.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| picked.contains(name))
        })
        .collect()
}

/// A hidden file a download writes next to its result. Results never start with a dot (`OutputNamePattern`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchFile {
    /// `.<job ID>.partial` and `.<job ID>.audio.mp4`.
    Job(Uuid),
    /// `.<UUID>.mp4` and `.<UUID>.m4a`, which the media port's audio restore writes while it runs.
    Unowned,
}

impl ScratchFile {
    /// The scratch file `name` is, if any.
    pub fn parse(name: &str) -> Option<Self> {
        let rest = name.strip_prefix('.')?;
        let (id, kind) = rest.split_once('.')?;
        // A UUID string as Foundation reads it: 36 characters with hyphens, in either case.
        if id.len() != 36 || id.matches('-').count() != 4 {
            return None;
        }
        let id = Uuid::try_parse(id).ok()?;
        match kind {
            "partial" | "audio.mp4" => Some(Self::Job(id)),
            "mp4" | "m4a" => Some(Self::Unowned),
            _ => None,
        }
    }

    /// The scratch files among `names` that no job can still use: those of jobs not in `kept`, and unowned ones
    /// unless a job runs, since its audio restore may be writing them.
    pub fn leftovers(names: &[String], kept: &HashSet<Uuid>, is_any_job_running: bool) -> Vec<String> {
        names
            .iter()
            .filter(|name| match Self::parse(name) {
                Some(Self::Job(id)) => !kept.contains(&id),
                Some(Self::Unowned) => !is_any_job_running,
                None => false,
            })
            .cloned()
            .collect()
    }
}
