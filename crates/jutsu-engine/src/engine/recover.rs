//! What a failed step does to its job: retry with backoff, pause, or fail.

use jutsu_core::{
    FailureReason, Job, JobEvent, JobState, PauseReason, ProviderError, RetryCause, RetryStatus,
};
use time::Duration;
use uuid::Uuid;

use super::{JobEngine, is_before_submit, is_queued_or_processing};
use crate::cancel;
use crate::error::{EngineError, Step, StepError};
use crate::settings::Waits;

impl JobEngine {
    /// Maps a failed step to the job's next state. Returns `false` when the runner should stop.
    pub(crate) async fn recover(&self, error: StepError, id: Uuid, delay: &mut Waits) -> bool {
        if cancel::is_cancelled() || matches!(error, StepError::Cancelled) {
            return false;
        }
        let Some(job) = self.job(id) else {
            return false;
        };
        let recorded: Step = async {
            match error {
                StepError::Provider(error) => self.recover_provider(error, &job, delay).await?,
                StepError::Engine(EngineError::InvalidTransition { .. }) => return Err(StepError::Cancelled),
                StepError::Engine(EngineError::NotConnected) if is_before_submit(&job.state) => {
                    self.apply(
                        JobEvent::Paused(PauseReason::ConnectionNeedsAttention),
                        id,
                        |_| {},
                    )
                    .await?;
                }
                error if error.is_out_of_space() && job.state == JobState::Downloading => {
                    // The result is paid for: the partial file and the request stay, so resuming continues the
                    // download.
                    tracing::error!(job = %id, ?error, "not enough disk space for the result");
                    self.apply(JobEvent::Paused(PauseReason::NotEnoughDiskSpace), id, |_| {})
                        .await?;
                }
                StepError::Failure(reason) => {
                    self.apply(JobEvent::Failed { reason }, id, |_| {}).await?;
                }
                error => {
                    tracing::error!(job = %id, ?error, "job failed");
                    let reason = error.failure_reason();
                    self.apply(JobEvent::Failed { reason }, id, |_| {}).await?;
                }
            }
            Ok(())
        }
        .await;
        match recorded {
            Ok(()) => true,
            Err(StepError::Cancelled) => false,
            Err(error) => {
                tracing::error!(job = %id, ?error, "can't record the error");
                false
            }
        }
    }

    async fn recover_provider(&self, error: ProviderError, job: &Job, delay: &mut Waits) -> Step {
        let id = job.id;
        match error {
            ProviderError::Network => self.retry_step(id, RetryCause::Network, delay, None).await,
            ProviderError::Server { status } => {
                self.retry_step(id, RetryCause::Server { status }, delay, None)
                    .await
            }
            ProviderError::ModelUnavailable => {
                self.retry_step(id, RetryCause::ModelUnavailable, delay, None)
                    .await
            }
            ProviderError::ConcurrencyLimit { retry_after } => {
                self.release_slot(id);
                if job.state == JobState::WaitingForSlot {
                    self.apply(JobEvent::HitConcurrencyLimit, id, |_| {}).await?;
                }
                self.retry_step(id, RetryCause::ConcurrencyLimit, delay, retry_after)
                    .await
            }
            ProviderError::Unauthorized => {
                // Submitted jobs keep their state and wait in `network_provider` for the connection.
                let handler = {
                    let mut state = self.st();
                    let handler = (!state.connection_needs_attention)
                        .then(|| state.connection_attention_handler.clone())
                        .flatten();
                    state.connection_needs_attention = true;
                    handler
                };
                if let Some(handler) = handler {
                    handler().await;
                }
                if is_before_submit(&job.state) {
                    self.apply(
                        JobEvent::Paused(PauseReason::ConnectionNeedsAttention),
                        id,
                        |_| {},
                    )
                    .await?;
                }
                Ok(())
            }
            ProviderError::InsufficientFunds => {
                if is_before_submit(&job.state) {
                    self.apply(JobEvent::Paused(PauseReason::InsufficientFunds), id, |_| {})
                        .await?;
                    Ok(())
                } else {
                    self.retry_step(id, RetryCause::InsufficientFunds, delay, None)
                        .await
                }
            }
            ProviderError::Validation { message, .. } => {
                self.apply(
                    JobEvent::Failed {
                        reason: FailureReason::Message(message),
                    },
                    id,
                    |_| {},
                )
                .await?;
                Ok(())
            }
            ProviderError::OutcomeUnknown => {
                self.apply(
                    JobEvent::Failed {
                        reason: FailureReason::OutcomeUnknown,
                    },
                    id,
                    |_| {},
                )
                .await?;
                Ok(())
            }
            ProviderError::Rejected => {
                let event = if is_queued_or_processing(&job.state) {
                    JobEvent::Rejected
                } else {
                    JobEvent::Failed {
                        reason: FailureReason::ContentModeration,
                    }
                };
                self.apply(event, id, |_| {}).await?;
                Ok(())
            }
        }
    }

    /// Waits before the failed step runs again, with the wait and the failure on the job (`Job::retrying`) so its
    /// row shows them. Waiting can't fix everything: a job not yet submitted that kept failing past the engine's
    /// patience pauses as `KeptFailing`, nothing charged, until the user resumes it; a busy account (concurrency
    /// limit) is waited for. A submitted job is never paused or resubmitted for it: it may still finish and be
    /// charged, so it retries until its generation deadline, as polling does, and a download until it succeeds.
    /// `server_wait` is the wait the server asked for (`Retry-After`), kept in `server_retry_delay`; jitter only
    /// lengthens it, within that range. Without it the wait is `delay`'s.
    async fn retry_step(
        &self,
        id: Uuid,
        cause: RetryCause,
        delay: &mut Waits,
        server_wait: Option<Duration>,
    ) -> Step {
        let job = self.job(id).ok_or(EngineError::UnknownJob(id))?;
        tracing::error!(job = %id, ?cause, "retrying");
        let settings = self.settings();
        let now = self.now();
        if job.retrying.is_none() {
            delay.reset();
        }
        let failures = job.retrying.as_ref().map_or(0, |retrying| retrying.failures) + 1;
        let since = job.retrying.as_ref().map_or(now, |retrying| retrying.since);
        if is_queued_or_processing(&job.state) && self.is_past_deadline(&job) {
            return self.fail_at_deadline(id).await;
        }
        if is_before_submit(&job.state)
            && cause != RetryCause::ConcurrencyLimit
            && failures >= settings.retry_pause_failures
            && now - since >= settings.retry_pause_after
        {
            tracing::error!(job = %id, seconds = (now - since).whole_seconds(), "kept failing, pausing");
            self.apply(JobEvent::Paused(PauseReason::KeptFailing), id, move |job| {
                job.retrying = Some(RetryStatus {
                    failures,
                    since,
                    next_attempt_at: now,
                    cause,
                });
            })
            .await?;
            return Ok(());
        }
        let wait = match server_wait {
            Some(asked) => {
                let (lower, upper) = settings.server_retry_delay;
                let asked = asked.clamp(lower, upper);
                let jitter = settings.jitter.clamp(0.0, 1.0);
                (asked * (1.0 + (self.0.random)() * jitter)).min(upper)
            }
            None => delay.next(),
        };
        self.save(id, move |job| {
            job.retrying = Some(RetryStatus {
                failures,
                since,
                next_attempt_at: now + wait,
                cause,
            });
        })
        .await?;
        self.sleep(wait).await
    }

    /// Whether a submitted job is past its generation deadline, which counts from submit or the last check again.
    fn is_past_deadline(&self, job: &Job) -> bool {
        let Some(started) = job.checked_again_at.or(job.submitted_at) else {
            return false;
        };
        self.now() - started >= self.settings().generation_deadline
    }

    /// Fails a submitted job without a result by its deadline. Its request stays, so it can be checked again.
    pub(crate) async fn fail_at_deadline(&self, id: Uuid) -> Step {
        tracing::error!(job = %id, "no result within the deadline");
        self.release_slot(id);
        let now = self.now();
        self.apply(
            JobEvent::Failed {
                reason: FailureReason::Deadline,
            },
            id,
            move |job| job.timed_out_at = Some(now),
        )
        .await?;
        Ok(())
    }
}
