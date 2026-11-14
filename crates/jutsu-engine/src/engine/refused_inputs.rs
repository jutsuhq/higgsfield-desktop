//! A submit refused for inputs that weren't uploaded: the job uploads them and sends the submit once more. Also the
//! check that keeps reused inputs on https.

use std::collections::HashSet;

use jutsu_core::{FailureReason, Job, JobSpec, ProviderError, RemoteInputs};
use url::Url;
use uuid::Uuid;

use super::JobEngine;
use crate::error::{Step, StepError};

/// Inputs naming nothing.
pub(crate) fn no_inputs() -> RemoteInputs {
    RemoteInputs {
        video: None,
        images: Vec::new(),
        audio: None,
    }
}

impl JobEngine {
    /// Records a submit that failed with `error` (anything but `OutcomeUnknown`). A refusal sent nothing to pay for,
    /// so the submit marker goes. A lost answer to a submit with the key keeps it: the request may be running, so it
    /// is resent with the same key and can't be cancelled as never sent. Returns `true` when the job was set to
    /// upload its inputs again (`uploads_after_refusal`).
    pub(crate) async fn settle_failed_submit(
        &self,
        error: &ProviderError,
        job: &Job,
        inputs: &RemoteInputs,
        is_idempotent: bool,
    ) -> Step<bool> {
        if !(is_idempotent && may_have_been_taken(error)) {
            self.save(job.id, |job| job.submit_attempt = None).await?;
        }
        // Only a refusal certainly sent nothing to pay for: an agent's reservation for it goes back to the budget.
        let reservation = self
            .job(job.id)
            .and_then(|job| job.agent.and_then(|agent| agent.reservation));
        if !may_have_been_taken(error)
            && let Some(reservation) = reservation
        {
            self.release_agent_spend(Some(reservation), job.id).await;
            self.save(job.id, |job| {
                if let Some(agent) = job.agent.as_mut() {
                    agent.reservation = None;
                }
            })
            .await?;
        }
        self.uploads_after_refusal(error, job, inputs).await
    }

    /// Whether a refused submit is sent again with uploaded inputs: reused media IDs, a result named by its job ID,
    /// media the account already keeps, or files sent by their link. Only a `Validation` refusal of those inputs
    /// counts, and each fallback happens once, since the job then forgets what it sent.
    pub(crate) async fn uploads_after_refusal(
        &self,
        error: &ProviderError,
        job: &Job,
        inputs: &RemoteInputs,
    ) -> Step<bool> {
        if !matches!(error, ProviderError::Validation { .. }) {
            return Ok(false);
        }
        if job.spec.remote_inputs.as_ref() == Some(inputs) {
            self.upload_again(job).await?;
        } else if names_result_by_job_id(&job.spec) && inputs.video.is_none() {
            self.upload_instead_of_job_id(job.id).await?;
        } else if Self::uses_server_media(inputs, &job.spec) {
            self.upload_instead_of_server_media(job.id).await?;
        } else if Self::uses_links(inputs, &job.spec) {
            self.upload_instead_of_linking(job.id).await?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// The provider refused media IDs reused from another generation, e.g. because it no longer keeps the files. The
    /// next submit downloads and uploads them again, once: the job stops reusing them.
    async fn upload_again(&self, job: &Job) -> Step {
        check_reused_inputs(&job.spec)?;
        tracing::error!(job = %job.id, "reused inputs were refused, uploading them again");
        self.release_slot(job.id);
        self.save(job.id, |job| {
            job.spec.remote_inputs = None;
            job.inputs = None;
            job.server_price = None;
        })
        .await
    }

    /// The provider refused the job ID an operation named its result by, e.g. because the tool takes only uploads.
    /// The next submit uploads the result's file instead, once: the job forgets the ID.
    async fn upload_instead_of_job_id(&self, id: Uuid) -> Step {
        tracing::error!(job = %id, "the result's job ID was refused, uploading the file instead");
        self.release_slot(id);
        self.save(id, |job| {
            if let Some(operation) = job.spec.operation.as_mut() {
                operation.source_job_id = None;
            }
            job.inputs = None;
            job.server_price = None;
        })
        .await
    }

    /// The provider refused the media IDs of an element's images, e.g. because the element was deleted. The next
    /// submit uploads the downloaded files instead, once: the job forgets the media.
    async fn upload_instead_of_server_media(&self, id: Uuid) -> Step {
        tracing::error!(job = %id, "the account's media IDs were refused, uploading the files instead");
        self.release_slot(id);
        self.save(id, |job| {
            for reference in &mut job.spec.references {
                reference.server_media = None;
            }
            job.inputs = None;
            job.server_price = None;
        })
        .await
    }

    /// The provider refused an input sent by its link, e.g. because the host doesn't serve it to Higgsfield. The
    /// next submit uploads the files instead, once: the job forgets where they came from.
    async fn upload_instead_of_linking(&self, id: Uuid) -> Step {
        tracing::error!(job = %id, "linked inputs were refused, uploading them instead");
        self.release_slot(id);
        self.save(id, |job| {
            if let Some(source) = job.spec.source.as_mut() {
                source.remote_origin = None;
            }
            for reference in &mut job.spec.references {
                reference.remote_origin = None;
            }
            job.inputs = None;
            job.server_price = None;
        })
        .await
    }
}

/// A failure that can come after the server took the request: no answer, or a server error.
pub(crate) fn may_have_been_taken(error: &ProviderError) -> bool {
    match error {
        ProviderError::Network | ProviderError::OutcomeUnknown => true,
        ProviderError::Server { status } => *status >= 500 || *status < 400,
        _ => false,
    }
}

/// An operation on a result the account made names it by its job ID, so nothing is prepared or uploaded.
pub(crate) fn names_result_by_job_id(spec: &JobSpec) -> bool {
    spec.remote_inputs.is_none()
        && spec
            .operation
            .as_ref()
            .is_some_and(|operation| operation.source_job_id.is_some())
}

/// Nothing is prepared or uploaded: an operation names its result by its job ID, and a draft's finalize names the
/// draft (`JobSpec::draft_job_id`), which no upload can stand in for.
pub(crate) fn uploads_nothing(spec: &JobSpec) -> bool {
    spec.draft_job_id.is_some() || names_result_by_job_id(spec)
}

impl JobEngine {
    /// Whether `inputs` name any reference by the media the account already keeps (`LocalImage::server_media`).
    pub(crate) fn uses_server_media(inputs: &RemoteInputs, spec: &JobSpec) -> bool {
        let ids: HashSet<&str> = spec
            .references
            .iter()
            .filter_map(|reference| reference.server_media.as_ref().map(|media| media.id.as_str()))
            .collect();
        inputs
            .images
            .iter()
            .any(|image| image.id.as_deref().is_some_and(|id| ids.contains(id)))
    }

    /// Whether `inputs` name any input by the link its file was downloaded from rather than by an upload.
    pub(crate) fn uses_links(inputs: &RemoteInputs, spec: &JobSpec) -> bool {
        let origins: HashSet<&Url> = spec
            .source
            .iter()
            .filter_map(|source| source.remote_origin.as_ref())
            .chain(
                spec.references
                    .iter()
                    .filter_map(|reference| reference.remote_origin.as_ref()),
            )
            .collect();
        inputs
            .video
            .as_ref()
            .is_some_and(|video| origins.contains(&video.url))
            || inputs.images.iter().any(|image| origins.contains(&image.url))
    }
}

/// Inputs reused from the provider are downloaded, never read as local files, so each must be https: a `file:` URL
/// among them came from the server, not from the user.
pub(crate) fn check_reused_inputs(spec: &JobSpec) -> Result<(), StepError> {
    let Some(reused) = &spec.remote_inputs else {
        return Ok(());
    };
    let mut urls = reused
        .video
        .iter()
        .map(|video| &video.url)
        .chain(spec.source.iter().map(|source| &source.file_url))
        .chain(reused.images.iter().map(|image| &image.url))
        .chain(spec.references.iter().map(|reference| &reference.file_url));
    if urls.all(higgsfield::core::is_https) {
        Ok(())
    } else {
        Err(StepError::Failure(FailureReason::InsecureInput))
    }
}
