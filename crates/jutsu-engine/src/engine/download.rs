//! A finished job's result on its way into the output folder, with its sound put back as the job asks.

use std::path::{Path, PathBuf};

use jutsu_core::{FailureReason, Job, JobEvent, OutputNamePattern, PortError, ProviderError, RemoteStatus};
use time::{Duration, OffsetDateTime};
use url::Url;

use super::leftovers::partial_file;
use super::{JobEngine, file_url};
use crate::cancel;
use crate::downloader::DownloadError;
use crate::error::{EngineError, Step, StepError};
use crate::file_replacement::FileReplacement;

/// How long Higgsfield keeps a finished result.
pub(crate) const RESULT_LIFETIME: Duration = Duration::days(7);

impl JobEngine {
    pub(crate) async fn download(&self, job: Job) -> Step {
        let known = self.st().video_urls.get(&job.id).cloned();
        let url = if let Some(url) = known {
            url
        } else {
            // After a relaunch or a retry: the status call returns the URL again, until the result expires.
            let status = match self.remote_status(&job).await {
                Err(StepError::Provider(ProviderError::Validation { .. })) => {
                    return Err(StepError::Failure(FailureReason::OutputUnavailable));
                }
                other => other?,
            };
            let RemoteStatus::Completed { video_url } = status else {
                return Err(StepError::Failure(FailureReason::OutputUnavailable));
            };
            video_url
        };
        self.wait_until_online().await?;
        let folder = if let Some(folder) = job
            .download_folder
            .as_ref()
            .and_then(|folder| folder.to_file_path().ok())
        {
            folder
        } else {
            let folder = self.settings().output_directory;
            let saved = file_url(&folder)?;
            self.save(job.id, move |job| job.download_folder = Some(saved))
                .await?;
            folder
        };
        std::fs::create_dir_all(&folder)?;
        let partial = partial_file(job.id, &folder);
        match self.fetch_result(&url, &partial, job.charged_at).await {
            Err(StepError::Provider(ProviderError::Server { status: 403 })) => {
                // A signed URL that expired: the next try asks the status call for a fresh one.
                self.st().video_urls.remove(&job.id);
                return Err(StepError::Provider(ProviderError::Server { status: 403 }));
            }
            other => other?,
        }
        // An operation's result keeps the sound the server gave it, in the container it came in, unless the server
        // drops the sound (`PostAction::restores_source_sound`): it is put back as for a generation, in an MP4.
        let restores_sound = job
            .spec
            .operation
            .as_ref()
            .is_none_or(|operation| operation.action.restores_source_sound());
        let finished = if restores_sound {
            self.restoring_audio(&job, &partial).await?
        } else {
            partial.clone()
        };
        let extension = (job.spec.operation.is_some() && finished == partial)
            .then(|| OutputNamePattern::result_extension(&url));
        let destination = self.place_result(&finished, &job, &folder, extension.as_deref())?;
        if finished != partial {
            let _ = std::fs::remove_file(&partial);
        }
        let destination = file_url(&destination)?;
        // What the job asked for isn't always what came back (a Seedance Edit ran shorter), so the file is read.
        let output_media = self.0.media.output_media(&destination).await;
        self.record(JobEvent::Downloaded, job.id, move |job| {
            job.output_file = Some(destination);
            job.output_media = output_media;
            job.progress = 1.0;
        })
        .await?;
        Ok(())
    }

    /// Downloads a finished result into `partial`. A refusal that waiting can fix fails with
    /// `ProviderError::Server` and keeps the partial file, so the runner retries with backoff and resumes: 408, 429,
    /// and 403 while the result is younger than `RESULT_LIFETIME` (a signed URL that expired). Any other refusal
    /// removes the partial file; 404, 410 and an old 403 mean the result expired. `finished_at` is when the
    /// generation finished; `None` when unknown, which counts as fresh.
    pub(crate) async fn fetch_result(
        &self,
        url: &Url,
        partial: &Path,
        finished_at: Option<OffsetDateTime>,
    ) -> Step {
        match cancel::cancellable(self.0.downloader.download(url, partial)).await? {
            Ok(()) => Ok(()),
            Err(DownloadError::Failed { status }) => {
                let is_fresh = finished_at.is_none_or(|finished| self.now() - finished < RESULT_LIFETIME);
                match status {
                    408 | 429 => Err(StepError::Provider(ProviderError::Server {
                        status: i64::from(status),
                    })),
                    403 if is_fresh => Err(StepError::Provider(ProviderError::Server {
                        status: i64::from(status),
                    })),
                    403 | 404 | 410 => {
                        let _ = std::fs::remove_file(partial);
                        Err(StepError::Failure(FailureReason::OutputUnavailable))
                    }
                    _ => {
                        let _ = std::fs::remove_file(partial);
                        Err(StepError::Provider(ProviderError::Validation {
                            field: None,
                            message: download_refused_reason(status),
                        }))
                    }
                }
            }
            Err(DownloadError::Refused(refusal)) => {
                let _ = std::fs::remove_file(partial);
                Err(StepError::Provider(ProviderError::Validation {
                    field: None,
                    message: refusal.message(),
                }))
            }
            Err(other) => Err(other.into()),
        }
    }

    /// The result with its sound as the job asks, or `partial` itself when nothing changes. Failing to add the sound
    /// keeps the silent result: the generation is paid for and still usable.
    async fn restoring_audio(&self, job: &Job, partial: &Path) -> Step<PathBuf> {
        let folder = partial.parent().unwrap_or(Path::new("."));
        let output = Self::audio_file(job.id, folder);
        let result = file_url(partial)?;
        let output_url = file_url(&output)?;
        match cancel::cancellable(self.0.media.restore_audio(&job.spec, &result, &output_url)).await? {
            Ok(true) => Ok(output),
            Ok(false) => Ok(partial.to_path_buf()),
            Err(PortError::Cancelled) => Err(StepError::Cancelled),
            Err(error) => {
                let error = StepError::from(error);
                if error.is_out_of_space() {
                    // Pauses the job, so the sound is still added once there is space again.
                    return Err(error);
                }
                tracing::error!(job = %job.id, ?error, "kept the result without source audio");
                Ok(partial.to_path_buf())
            }
        }
    }

    /// Moves a finished result into `directory` under the job's name, next to any file already there.
    /// `extension` is the result's own extension; `None` for `OutputNamePattern::FILE_EXTENSION`.
    pub(crate) fn place_result(
        &self,
        file: &Path,
        job: &Job,
        directory: &Path,
        extension: Option<&str>,
    ) -> Result<PathBuf, EngineError> {
        let settings = self.settings();
        let preset_name = job.spec.preset_id.and_then(|id| (self.0.preset_name)(id));
        let name = settings.output_name_pattern.file_name(
            job,
            preset_name.as_deref(),
            settings.utc_offset,
            extension,
        );
        let free = OutputNamePattern::available(&name, |candidate| directory.join(candidate).exists());
        place(file, &directory.join(free))
    }
}

/// Moves the finished download to `destination`. An earlier file there stays until the swap succeeds.
fn place(partial: &Path, destination: &Path) -> Result<PathBuf, EngineError> {
    FileReplacement::move_file(partial, destination).map_err(|error| EngineError::io(&error))?;
    Ok(destination.to_path_buf())
}

pub(crate) fn download_refused_reason(status: u16) -> String {
    format!("Higgsfield refused to send the result (HTTP {status}). Retry to download it again.")
}
