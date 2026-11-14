//! A job's inputs on their way to the provider: downloaded when only remote, prepared, then uploaded or linked.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_core::{
    FailureReason, Job, JobState, LocalImage, MediaLinkKind, PreparedFile, ProviderError, ProviderKind,
    RemoteAsset, RemoteInputs,
};
use url::Url;
use uuid::Uuid;

use super::refused_inputs::names_result_by_job_id;
use super::{JobEngine, PreparedInputs, file_url};
use crate::cancel;
use crate::downloader::DownloadError;
use crate::error::{Step, StepError};

/// Removes the inputs downloaded for a `prepare` once it returns.
struct Downloaded(Vec<PathBuf>);

impl Drop for Downloaded {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

impl JobEngine {
    /// Inputs that are only on the provider's side, from a generation made elsewhere, are downloaded first.
    pub(crate) async fn prepare(&self, job: &Job) -> Step<PreparedInputs> {
        let mut downloaded = Downloaded(Vec::new());
        let mut video = None;
        if let Some(mut source) = job.spec.source.clone() {
            if source.file_url.scheme() != "file" {
                let file = self.fetch_input(&source.file_url, "source", job.id).await?;
                source.file_url = file_url(&file)?;
                downloaded.0.push(file);
            }
            let engine = self.clone();
            let id = job.id;
            let progress = Arc::new(move |fraction: f64| engine.report(fraction, id, &JobState::Preparing));
            video = Some(cancel::cancellable(self.0.media.prepare_video(&source, progress)).await??);
        }
        let mut images = Vec::new();
        for (index, reference) in job.spec.references.iter().enumerate() {
            let mut image = reference.clone();
            if image.file_url.scheme() != "file" {
                let file = self
                    .fetch_input(&image.file_url, &format!("image-{index}"), job.id)
                    .await?;
                image = LocalImage::new(file_url(&file)?);
                downloaded.0.push(file);
            }
            images.push(cancel::cancellable(self.0.media.prepare_image(&image)).await??);
        }
        let mut audio = None;
        if let Some(mut speech) = job.spec.speech.clone() {
            if speech.file_url.scheme() != "file" {
                let file = self.fetch_input(&speech.file_url, "speech", job.id).await?;
                speech.file_url = file_url(&file)?;
                downloaded.0.push(file);
            }
            audio = Some(cancel::cancellable(self.0.media.prepare_audio(&speech)).await??);
        }
        Ok(PreparedInputs { video, images, audio })
    }

    /// Downloads a remote input into the temporary folder; an earlier partial download there is resumed.
    async fn fetch_input(&self, url: &Url, name: &str, id: Uuid) -> Step<PathBuf> {
        self.wait_until_online().await?;
        let extension = jutsu_core::support::url_ext::url_extension(url);
        let extension = if extension.is_empty() {
            "bin".to_owned()
        } else {
            extension
        };
        let file = std::env::temp_dir().join(format!(
            "jutsu-{}-{name}.{extension}",
            jutsu_core::support::uuid_string(id)
        ));
        match cancel::cancellable(self.0.downloader.download(url, &file)).await? {
            Ok(()) => Ok(file),
            Err(DownloadError::Failed { .. }) => Err(StepError::Failure(FailureReason::InputUnavailable)),
            Err(DownloadError::Refused(refusal)) => Err(StepError::Provider(ProviderError::Validation {
                field: None,
                message: refusal.message(),
            })),
            Err(other) => Err(other.into()),
        }
    }

    pub(crate) async fn upload(&self, job: &Job) -> Step<RemoteInputs> {
        if job.spec.draft_job_id.is_some() {
            return Ok(super::refused_inputs::no_inputs());
        }
        if let Some(reused) = &job.spec.remote_inputs {
            return Ok(reused.clone());
        }
        if names_result_by_job_id(&job.spec) {
            return Ok(super::refused_inputs::no_inputs());
        }
        let cached = self.st().prepared.get(&job.id).cloned();
        let files = match cached {
            Some(files) => files,
            None => self.prepare(job).await?,
        };
        let video_count = usize::from(files.video.is_some());
        let audio_count = usize::from(files.audio.is_some());
        #[allow(clippy::cast_precision_loss, reason = "a handful of inputs")]
        let total = (files.images.len() + video_count + audio_count) as f64;
        let mut video = None;
        if let Some(file) = files.video {
            let source = job.spec.source.as_ref();
            let linked = self
                .linked(
                    source.and_then(|source| source.linkable_origin()),
                    source.map(|source| &source.file_url),
                    MediaLinkKind::Video,
                    job,
                )
                .await?;
            video = Some(match linked {
                Some(asset) => asset,
                None => self.upload_file(file, job).await?,
            });
            self.report(1.0 / total, job.id, &JobState::Uploading);
        }
        let mut images = Vec::new();
        for (index, file) in files.images.into_iter().enumerate() {
            let reference = job.spec.references.get(index);
            let mut asset = None;
            if let Some(media) = reference.and_then(|reference| reference.server_media.as_ref()) {
                let provider = self.network_provider(job).await?;
                asset = cancel::cancellable(provider.server_asset(media)).await??;
            }
            if asset.is_none() {
                asset = self
                    .linked(
                        reference.and_then(LocalImage::linkable_origin),
                        reference.map(|reference| &reference.file_url),
                        MediaLinkKind::Image,
                        job,
                    )
                    .await?;
            }
            images.push(match asset {
                Some(asset) => asset,
                None => self.upload_file(file, job).await?,
            });
            #[allow(clippy::cast_precision_loss, reason = "a handful of inputs")]
            let done = (images.len() + video_count) as f64;
            self.report(done / total, job.id, &JobState::Uploading);
        }
        let mut audio = None;
        if let Some(file) = files.audio {
            audio = Some(self.upload_file(file, job).await?);
            self.report(1.0, job.id, &JobState::Uploading);
        }
        self.st().prepared.remove(&job.id);
        Ok(RemoteInputs { video, images, audio })
    }

    async fn upload_file(&self, file: PreparedFile, job: &Job) -> Step<RemoteAsset> {
        let cutoff = self.now() - self.settings().upload_max_age;
        let key = upload_cache_key(&file.content_hash, job.provider_kind, job.owner.as_deref());
        if let Some(asset) = cancel::cancellable(self.0.upload_cache.asset(&key, cutoff)).await?? {
            return Ok(asset);
        }
        let provider = self.network_provider(job).await?;
        let asset = cancel::cancellable(provider.upload(file)).await??;
        cancel::cancellable(self.0.upload_cache.store(&asset, &key, self.now())).await??;
        Ok(asset)
    }

    /// The input as the provider takes it from the link its file was downloaded from, so it isn't uploaded; `None`
    /// when there is no such link (`SourceClip::linkable_origin`), the file's size can't be read or the provider
    /// needs an upload.
    async fn linked(
        &self,
        origin: Option<&Url>,
        file: Option<&Url>,
        kind: MediaLinkKind,
        job: &Job,
    ) -> Step<Option<RemoteAsset>> {
        let (Some(origin), Some(file)) = (origin, file) else {
            return Ok(None);
        };
        let Some(size) = file
            .to_file_path()
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .and_then(|metadata| i64::try_from(metadata.len()).ok())
        else {
            return Ok(None);
        };
        let provider = self.network_provider(job).await?;
        Ok(cancel::cancellable(provider.linked_asset(origin, kind, size)).await??)
    }
}

/// Uploads are reused only by the account that made them.
pub(crate) fn upload_cache_key(content_hash: &str, kind: ProviderKind, owner: Option<&str>) -> String {
    format!("{}|{}|{content_hash}", kind.raw_value(), owner.unwrap_or(""))
}
