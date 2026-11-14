//! Results of generations made elsewhere, brought into the output folder.

use futures::FutureExt;
use futures::future::BoxFuture;
use jutsu_core::Job;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::leftovers::partial_file;
use super::{JobEngine, file_url};
use crate::cancel;
use crate::error::{EngineError, Step};

/// Takes the running import off the engine's list once its starter is done with it.
struct ImportFinish {
    engine: JobEngine,
    id: uuid::Uuid,
}

impl Drop for ImportFinish {
    fn drop(&mut self) {
        self.engine.st().importing.remove(&self.id);
    }
}

impl JobEngine {
    /// Downloads the output of a generation made elsewhere into the output folder and saves `job`, a finished job
    /// from `Job::importing`, with it and the file's own length and frame size (`Job::output_media`): the provider's
    /// params for them are only a hint, and can be wrong. Importing a job that is already stored returns the stored
    /// one, and importing one whose import is still running waits for that one instead of downloading it a second
    /// time. A refusal worth waiting out (`fetch_result`) fails with `ProviderError::Server`; importing again resumes,
    /// until the next `start` deletes the partial file. Dropping the import that started the download cancels it.
    pub async fn import_finished(&self, job: Job, url: Url) -> Result<Job, EngineError> {
        let running = self.st().importing.get(&job.id).cloned();
        if let Some(running) = running {
            return running.await;
        }
        if let Some(stored) = self.0.repository.job(job.id).await? {
            return Ok(stored);
        }
        let id = job.id;
        let token = CancellationToken::new();
        let started = 'started: {
            let mut state = self.st();
            // Another import may have started while the store was read.
            if let Some(running) = state.importing.get(&id).cloned() {
                break 'started Err(running);
            }
            let engine = self.clone();
            let fetch: BoxFuture<'static, Step<Job>> =
                Box::pin(async move { engine.fetch_import(job, url).await });
            let handle = tokio::spawn(cancel::scope(token.clone(), fetch));
            let shared = async move {
                match handle.await {
                    Ok(result) => result.map_err(EngineError::from),
                    Err(_) => Err(EngineError::Cancelled),
                }
            }
            .boxed()
            .shared();
            state.importing.insert(id, shared.clone());
            Ok(shared)
        };
        let shared = match started {
            Ok(shared) => shared,
            Err(running) => return running.await,
        };
        let _finish = ImportFinish {
            engine: self.clone(),
            id,
        };
        let _cancel = token.drop_guard();
        shared.await
    }

    async fn fetch_import(&self, job: Job, url: Url) -> Step<Job> {
        self.wait_until_online().await?;
        let folder = self.settings().output_directory;
        std::fs::create_dir_all(&folder)?;
        let partial = partial_file(job.id, &folder);
        self.fetch_result(&url, &partial, job.charged_at).await?;
        let mut imported = job;
        let file = file_url(&self.place_result(&partial, &imported, &folder, None)?)?;
        imported.output_media = self.0.media.output_media(&file).await;
        imported.output_file = Some(file);
        cancel::cancellable(self.0.repository.save(&imported)).await??;
        Ok(imported)
    }
}
