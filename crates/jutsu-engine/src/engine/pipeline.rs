//! One job's way through the pipeline: prepare → upload → slot → submit → poll → download.

use jutsu_core::{
    Budget, Cost, FailureReason, Job, JobEvent, JobState, PauseReason, PortError, RemoteInputs, RemoteJob,
    RemoteStatus, SubmitAttempt,
};
use uuid::Uuid;

use super::agent_guard::AgentClearance;
use super::refused_inputs::{check_reused_inputs, uploads_nothing};
use super::status_batch::BatchedStatus;
use super::{JobEngine, is_queued_or_processing, is_runnable};
use crate::cancel;
use crate::error::{EngineError, Step, StepError};
use crate::settings::Waits;

/// Ends a submit begun with `begin_submit` when dropped, whatever way the submit ends.
struct SubmitFinish {
    engine: JobEngine,
    id: Uuid,
}

impl Drop for SubmitFinish {
    fn drop(&mut self) {
        self.engine.st().submits.finish(self.id);
    }
}

impl JobEngine {
    pub(crate) async fn drive(&self, id: Uuid, token: Uuid) {
        let mut retry_delay = Waits::retrying(&self.settings(), self.0.random.clone());
        loop {
            if cancel::is_cancelled() {
                break;
            }
            let Some(job) = self.job(id).filter(|job| is_runnable(&job.state)) else {
                break;
            };
            match Box::pin(self.step(job)).await {
                Ok(()) => retry_delay.reset(),
                Err(error) => {
                    if !Box::pin(self.recover(error, id, &mut retry_delay)).await {
                        break;
                    }
                }
            }
        }
        self.release_slot(id);
        let mut state = self.st();
        state.prepared.remove(&id);
        state.video_urls.remove(&id);
        if state.runners.get(&id).is_some_and(|runner| runner.token == token) {
            state.runners.remove(&id);
        }
    }

    /// Does the work of the job's current state and moves it on.
    async fn step(&self, job: Job) -> Step {
        match job.state {
            JobState::Preparing => {
                check_reused_inputs(&job.spec)?;
                let uploads = job.spec.remote_inputs.is_none() && !uploads_nothing(&job.spec);
                let prepared = if uploads {
                    Some(self.prepare(&job).await?)
                } else {
                    None
                };
                {
                    let mut state = self.st();
                    match prepared {
                        Some(prepared) => state.prepared.insert(job.id, prepared),
                        None => state.prepared.remove(&job.id),
                    };
                }
                self.apply(JobEvent::Prepared, job.id, |_| {}).await?;
            }
            JobState::Uploading => {
                let inputs = self.upload(&job).await?;
                self.apply(JobEvent::Uploaded, job.id, move |job| job.inputs = Some(inputs))
                    .await?;
            }
            JobState::WaitingForSlot => Box::pin(self.submit(job)).await?,
            JobState::Queued | JobState::Processing => self.poll(job.id).await?,
            JobState::Downloading => self.download(job).await?,
            JobState::Done
            | JobState::Failed { .. }
            | JobState::Rejected
            | JobState::Canceled
            | JobState::Paused(_) => {}
        }
        Ok(())
    }

    /// Resubmits always carry the job's one idempotency key, so a lost response can't charge twice. A provider that
    /// can't send it gets one submit: an attempt still marked after a quit, crash or pause fails as unknown.
    async fn submit(&self, job: Job) -> Step {
        let id = job.id;
        if job.submit_attempt.is_some_and(|attempt| !attempt.is_idempotent) {
            self.apply(
                JobEvent::Failed {
                    reason: FailureReason::SubmitCutOff,
                },
                id,
                |_| {},
            )
            .await?;
            return Ok(());
        }
        let inputs = match job.inputs.clone() {
            Some(saved) => saved,
            None => self.upload(&job).await?,
        };
        if !Box::pin(self.price_fits_cap(id, &inputs)).await? {
            return Ok(());
        }
        self.acquire_slot(id).await?;
        let provider = self.network_provider(&job).await?;
        let is_idempotent = cancel::answered(provider.submit_is_idempotent(&job.spec)).await??;
        // A pause or cancel that lands here must not leave a submit marker for a request never sent.
        if cancel::is_cancelled() {
            return Err(StepError::Cancelled);
        }
        // An agent's job passes its spending guard and reserves its price before anything is sent.
        let AgentClearance::Send { reservation } = self.clear_agent_spend(id, &inputs).await? else {
            return Ok(());
        };
        // A stopping engine sends nothing new; the job submits on the next start.
        self.begin_submit(id, is_idempotent, reservation).await?;
        let _finish = SubmitFinish {
            engine: self.clone(),
            id,
        };
        // An answer that arrives as a cancel or a pause lands names a paid request: it is recorded, not dropped.
        let answer = cancel::answered(provider.submit(&job.spec, &inputs, job.idempotency_key)).await?;
        let mut remote = match answer {
            Ok(remote) => remote,
            Err(PortError::Provider(error)) if error != jutsu_core::ProviderError::OutcomeUnknown => {
                if self
                    .settle_failed_submit(&error, &job, &inputs, is_idempotent)
                    .await?
                {
                    return Ok(());
                }
                return Err(StepError::Provider(error));
            }
            Err(other) => return Err(other.into()),
        };
        let initial_status = remote.initial_status.take();
        let paid_with_unlimited = std::mem::take(&mut remote.paid_with_unlimited);
        let charged = remote.charged.take();
        let variants: Vec<RemoteJob> = std::mem::take(&mut remote.variants);
        // The job as saved now, with the server price `price_fits_cap` may have added. Every job the answer names is
        // paid for, so they are tracked even if that copy is gone.
        let sender = self.job(id).unwrap_or_else(|| job.clone());
        let share = if sender.spec.variants > 1 || !variants.is_empty() {
            Some(
                self.start_variants(&variants, &sender, &inputs, paid_with_unlimited)
                    .await,
            )
        } else {
            None
        };
        // Sent for the allowance and not paid by it: the charge counts, or the job's price when the answer can't say.
        let unpaid = sender.spec.uses_unlimited && !paid_with_unlimited;
        let counted = charged.clone().or_else(|| unpaid.then(|| sender.price().clone()));
        self.settle_agent_spend(&sender, counted.as_ref()).await;
        let now = self.now();
        self.record(JobEvent::Submitted, id, move |job| {
            if let Some(share) = &share {
                share.apply(job);
            }
            // Sent without the allowance, or the answer says it cost credits: the cap counts it from now on.
            if !paid_with_unlimited {
                job.spec.uses_unlimited = false;
            }
            if let Some(charged) = charged {
                job.server_price = Some(charged);
            }
            job.inputs = Some(inputs);
            job.remote = Some(remote);
            job.submit_attempt = None;
            job.submitted_at = Some(now);
        })
        .await?;
        // A submit answered with a final state needs no poll to act on it.
        if let Some(status) = initial_status {
            self.handle(status, id).await?;
        }
        Ok(())
    }

    /// Asks the provider for its price of the exact request before it is paid for, then checks it against the
    /// daily cap. Returns `false` when the job paused because that price needs the user. Without a quote the
    /// preliminary price stands. A submit that may already have reached the server is resent unchecked, and a job
    /// paid from the unlimited allowance isn't quoted: the cap doesn't count it.
    async fn price_fits_cap(&self, id: Uuid, inputs: &RemoteInputs) -> Step<bool> {
        let job = self.load_job(id).await?;
        if job.submit_attempt.is_some() || job.spec.uses_unlimited {
            return Ok(true);
        }
        let job = self.add_server_price(job, inputs).await?;
        let status = self.spend_status(&job).await?;
        let (Some(price), Some(status)) = (job.server_price.as_ref(), status) else {
            return Ok(true);
        };
        if !Budget::refined_price_needs_confirmation(
            &job.cost,
            price,
            job.approved_price.as_ref(),
            status.committed_today,
            status.cap.as_ref(),
        ) {
            return Ok(true);
        }
        let inputs = inputs.clone();
        self.apply(JobEvent::Paused(PauseReason::PriceAboveCap), id, move |job| {
            job.inputs = Some(inputs);
        })
        .await?;
        Ok(false)
    }

    /// The provider's price of the exact request, paid in credits or dollars rather than the plan's unlimited
    /// allowance, saved as the job's `server_price`. A job that has one keeps it; without a quote it is returned as
    /// it was and the preliminary price stands.
    pub(crate) async fn add_server_price(&self, mut job: Job, inputs: &RemoteInputs) -> Step<Job> {
        if job.server_price.is_some() {
            return Ok(job);
        }
        let mut spec = job.spec.clone();
        spec.uses_unlimited = false;
        let provider = self.network_provider(&job).await?;
        match cancel::cancellable(provider.server_estimate(&spec, inputs)).await? {
            Ok(Some(price)) => {
                let saved = price.clone();
                self.save(job.id, move |job| job.server_price = Some(saved))
                    .await?;
                job.server_price = Some(price);
            }
            Ok(None) => {}
            Err(PortError::Cancelled) => return Err(StepError::Cancelled),
            Err(error) => {
                tracing::error!(job = %job.id, %error, "no server estimate, keeping the preliminary price");
            }
        }
        Ok(job)
    }

    /// Unknown statuses read as in progress, so only the deadline ends a job the provider never finishes. It fails
    /// without a resubmit: the generation may still finish, and be charged, on the provider's side.
    async fn poll(&self, id: Uuid) -> Step {
        self.occupy_slot(id);
        let settings = self.settings();
        let mut delay = Waits::polling(&settings, self.0.random.clone());
        let mut last: Option<RemoteStatus> = None;
        let started = self
            .job(id)
            .and_then(|job| job.checked_again_at.or(job.submitted_at))
            .unwrap_or_else(|| self.now());
        while let Some(job) = self.job(id).filter(|job| is_queued_or_processing(&job.state)) {
            let status = if self.polls_in_batch(&job) {
                match self.batched_status(id).await? {
                    BatchedStatus::Status(answer) => answer,
                    BatchedStatus::Alone => self.remote_status(&job).await?,
                }
            } else {
                self.sleep(delay.next()).await?;
                self.remote_status(&job).await?
            };
            if self.job(id).is_some_and(|job| job.retrying.is_some()) {
                self.save(id, |job| job.retrying = None).await?;
            }
            if last.as_ref() != Some(&status) {
                delay.reset();
            }
            last = Some(status.clone());
            self.handle(status, id).await?;
            if self
                .job(id)
                .is_some_and(|job| is_queued_or_processing(&job.state))
                && self.now() - started >= settings.generation_deadline
            {
                self.fail_at_deadline(id).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn remote_status(&self, job: &Job) -> Step<RemoteStatus> {
        let Some(remote) = &job.remote else {
            return Err(StepError::Engine(EngineError::MissingRequestId));
        };
        let provider = self.network_provider(job).await?;
        Ok(cancel::cancellable(provider.status(remote)).await??)
    }

    pub(crate) async fn handle(&self, status: RemoteStatus, id: Uuid) -> Step {
        match status {
            RemoteStatus::Queued => {}
            RemoteStatus::InProgress => {
                // A job checked again after the deadline keeps the start an earlier poll saw.
                if self.job(id).is_some_and(|job| job.state == JobState::Queued) {
                    let now = self.now();
                    self.apply(JobEvent::Started, id, move |job| {
                        job.processing_at = Some(job.processing_at.unwrap_or(now));
                    })
                    .await?;
                }
            }
            RemoteStatus::Completed { video_url } => {
                self.st().video_urls.insert(id, video_url);
                self.release_slot(id);
                let now = self.now();
                self.apply(JobEvent::Completed, id, move |job| job.charged_at = Some(now))
                    .await?;
                self.read_charge(id).await?;
            }
            // Higgsfield doesn't bill a generation that failed, was refused or was cancelled: the agent's share goes
            // back.
            RemoteStatus::Failed { message } => {
                self.release_slot(id);
                let reason = message.map_or(FailureReason::GenerationFailed, FailureReason::Message);
                self.apply(JobEvent::Failed { reason }, id, |_| {}).await?;
                self.unbill_agent_spend(id).await;
            }
            RemoteStatus::Rejected(rejection) => {
                self.release_slot(id);
                self.apply(JobEvent::Rejected, id, move |job| job.rejection = Some(rejection))
                    .await?;
                self.unbill_agent_spend(id).await;
            }
            RemoteStatus::Canceled => {
                self.release_slot(id);
                let cancellable = self.job(id).is_some_and(|job| job.state.is_cancellable());
                let event = if cancellable {
                    JobEvent::Cancel
                } else {
                    JobEvent::Failed {
                        reason: FailureReason::CanceledOnServer,
                    }
                };
                self.apply(event, id, |_| {}).await?;
                self.unbill_agent_spend(id).await;
            }
        }
        Ok(())
    }

    /// A job queued without a price (`Job::price_is_unknown`, a Topaz upscale) gets what the provider says it charged
    /// once it has finished. Without an answer, or when the answer fails, the price stays unknown and the download
    /// goes on.
    async fn read_charge(&self, id: Uuid) -> Step {
        let Some(job) = self.job(id).filter(Job::price_is_unknown) else {
            return Ok(());
        };
        let Ok(provider) = self.current_provider(&job) else {
            return Ok(());
        };
        let charged: Option<Cost> = match cancel::cancellable(provider.charge(&job)).await? {
            Ok(charged) => charged,
            Err(PortError::Cancelled) => return Err(StepError::Cancelled),
            Err(error) => {
                tracing::error!(job = %id, %error, "couldn't read what it was charged");
                return Ok(());
            }
        };
        if let Some(charged) = charged {
            self.save(id, move |job| job.server_price = Some(charged)).await?;
        }
        Ok(())
    }

    /// Marks the submit as begun and saves its marker with the agent's reservation. A stopping engine sends nothing
    /// new, and anything that stops the send here releases the reservation, as nothing was sent. A reservation of an
    /// earlier submit the job still holds keeps counting (`keep_agent_spend`). The caller finishes the submit once
    /// this returns.
    async fn begin_submit(&self, id: Uuid, is_idempotent: bool, reservation: Option<Uuid>) -> Step {
        let begun = !cancel::is_cancelled() && self.st().submits.begin(id);
        if !begun {
            self.release_agent_spend(reservation, id).await;
            return Err(StepError::Cancelled);
        }
        let marked = async {
            let earlier = self
                .job(id)
                .and_then(|job| job.agent.and_then(|agent| agent.reservation));
            if let (Some(reservation), Some(earlier)) = (reservation, earlier)
                && earlier != reservation
            {
                self.keep_agent_spend(Some(earlier), id).await?;
            }
            self.save(id, move |job| {
                job.submit_attempt = Some(SubmitAttempt { is_idempotent });
                if let Some(reservation) = reservation
                    && let Some(agent) = job.agent.as_mut()
                {
                    agent.reservation = Some(reservation);
                }
            })
            .await
        }
        .await;
        if let Err(error) = marked {
            self.st().submits.finish(id);
            self.release_agent_spend(reservation, id).await;
            return Err(error);
        }
        Ok(())
    }
}
