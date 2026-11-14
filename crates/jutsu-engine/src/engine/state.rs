//! Transitions, progress, the network gate and slot bookkeeping.

use std::sync::Arc;

use jutsu_core::{FailureReason, GenerationProvider, Job, JobEvent, JobState, SpendStatus};
use time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{EngineEvent, JobEngine, is_before_submit};
use crate::cancel;
use crate::error::{EngineError, Step, StepError};
use crate::slot_limiter::SlotOrder;

/// A job's write turn; ends when dropped.
pub(crate) struct WriteTurn {
    engine: JobEngine,
    id: Uuid,
}

impl Drop for WriteTurn {
    fn drop(&mut self) {
        self.engine.st().write_turns.end(self.id);
    }
}

impl JobEngine {
    /// Moves the job through `JobState::transition`, saves it, then publishes it. An impossible event is a bug: it
    /// is logged and fails the job.
    pub(crate) async fn apply(
        &self,
        event: JobEvent,
        id: Uuid,
        update: impl FnOnce(&mut Job) + Send,
    ) -> Step<Job> {
        if cancel::is_cancelled() {
            return Err(StepError::Cancelled);
        }
        self.record(event, id, update).await
    }

    /// `apply` for a step that already happened, such as a submit the server answered or a result placed in the
    /// output folder: it is written even when the runner was cancelled meanwhile, so the job keeps the paid request
    /// or the file instead of losing track of it.
    pub(crate) async fn record(
        &self,
        event: JobEvent,
        id: Uuid,
        update: impl FnOnce(&mut Job) + Send,
    ) -> Step<Job> {
        let _turn = self.write_turn(id).await;
        let job = self.job(id).ok_or(EngineError::UnknownJob(id))?;
        self.transition(job, event, update).await
    }

    /// `apply` for a change decided on a copy of the job read before a suspension, such as the agent raising its
    /// `max_cost` or the person's Resume: `still_holds` is checked again on the job as last saved, within its write
    /// turn, and a job another writer moved on meanwhile is left as it is (`None`) instead of rejected as an
    /// impossible event.
    pub(crate) async fn apply_if_still(
        &self,
        event: JobEvent,
        id: Uuid,
        still_holds: impl FnOnce(&Job) -> bool + Send,
        update: impl FnOnce(&mut Job) + Send,
    ) -> Step<Option<Job>> {
        if cancel::is_cancelled() {
            return Err(StepError::Cancelled);
        }
        let _turn = self.write_turn(id).await;
        let job = self.job(id).ok_or(EngineError::UnknownJob(id))?;
        if !still_holds(&job) {
            return Ok(None);
        }
        self.transition(job, event, update).await.map(Some)
    }

    /// Moves `job` through `event` and saves it; runs within the job's write turn.
    async fn transition(
        &self,
        mut job: Job,
        event: JobEvent,
        update: impl FnOnce(&mut Job) + Send,
    ) -> Step<Job> {
        let Some(next) = JobState::transition(&job.state, &event) else {
            let from = job.state.clone();
            self.reject_transition(&event, job).await;
            return Err(StepError::Engine(EngineError::InvalidTransition { from, event }));
        };
        let partial = self.partial_download(job.id);
        // A new state is a step that got through; the concurrency limit keeps a job waiting for a slot.
        if next != job.state {
            job.retrying = None;
        }
        let terminal = next.is_terminal();
        job.state = next;
        job.progress = 0.0;
        job.finished_at = terminal.then(|| self.now());
        if terminal {
            job.download_folder = None;
        }
        update(&mut job);
        self.0.repository.save(&job).await?;
        self.st().jobs.insert(job.id, job.clone());
        self.publish(&job);
        if terminal {
            let _ = std::fs::remove_file(partial);
        }
        Ok(job)
    }

    /// Saves a change that keeps the job's state, then publishes it.
    pub(crate) async fn save(&self, id: Uuid, update: impl FnOnce(&mut Job) + Send) -> Step {
        let _turn = self.write_turn(id).await;
        let mut job = self.job(id).ok_or(EngineError::UnknownJob(id))?;
        update(&mut job);
        self.0.repository.save(&job).await?;
        self.st().jobs.insert(id, job.clone());
        self.publish(&job);
        Ok(())
    }

    /// Waits until no other change of job `id` is being saved. Each change then starts from the last saved one: two
    /// writers that both copied the job before either saved would otherwise drop each other's fields.
    async fn write_turn(&self, id: Uuid) -> WriteTurn {
        loop {
            let (sender, turn) = oneshot::channel();
            {
                let mut state = self.st();
                if state.write_turns.take(id) {
                    break;
                }
                state.write_turns.wait(id, sender);
            }
            let _ = turn.await;
        }
        WriteTurn {
            engine: self.clone(),
            id,
        }
    }

    async fn reject_transition(&self, event: &JobEvent, mut job: Job) {
        // The states can hold a failure's text, with paths or server words in it.
        tracing::error!(job = %job.id, ?event, state = ?job.state, "impossible transition");
        let Some(failed) = JobState::transition(
            &job.state,
            &JobEvent::Failed {
                reason: FailureReason::InternalError,
            },
        ) else {
            return;
        };
        job.state = failed;
        job.finished_at = Some(self.now());
        if let Err(error) = self.0.repository.save(&job).await {
            tracing::error!(job = %job.id, %error, "saving failed");
        }
        self.st().jobs.insert(job.id, job.clone());
        self.publish(&job);
    }

    /// Progress isn't saved: it only matters while the app runs. Small steps are kept without an event
    /// (`ProgressCoalescer`); the next published change of the job carries them.
    pub(crate) fn report(&self, progress: f64, id: Uuid, in_state: &JobState) {
        let now = self.now();
        let mut state = self.st();
        let Some(job) = state.jobs.get_mut(&id).filter(|job| job.state == *in_state) else {
            return;
        };
        job.progress = progress;
        if state.progress_coalescer.should_publish(id, progress, now) {
            state.progress_coalescer.did_publish(id, progress, now);
            let _ = self.0.events.send(EngineEvent::Progress {
                job_id: id,
                state: in_state.clone(),
                fraction: progress,
            });
        }
    }

    /// Sends `job` to the events. Every change goes through here so progress coalescing knows what the UI has.
    pub(crate) fn publish(&self, job: &Job) {
        let now = self.now();
        {
            let mut state = self.st();
            if job.state.is_terminal() {
                state.progress_coalescer.forget(job.id);
            } else {
                state.progress_coalescer.did_publish(job.id, job.progress, now);
            }
        }
        let _ = self.0.events.send(EngineEvent::JobChanged(Box::new(job.clone())));
    }

    /// The provider to use for `job` right now, without waiting.
    pub(crate) fn current_provider(&self, job: &Job) -> Result<Arc<dyn GenerationProvider>, EngineError> {
        let state = self.st();
        if !state.is_online {
            return Err(EngineError::Offline);
        }
        match &state.provider {
            Some(provider) if provider.owns(job) && !state.connection_needs_attention => Ok(provider.clone()),
            _ => Err(EngineError::NotConnected),
        }
    }

    /// Waits until the network and the job's connection are usable. An unsubmitted job without a usable connection
    /// fails with `NotConnected` so it pauses; a submitted one waits for the connection.
    pub(crate) async fn network_provider(&self, job: &Job) -> Step<Arc<dyn GenerationProvider>> {
        loop {
            if cancel::is_cancelled() {
                return Err(StepError::Cancelled);
            }
            let token = Uuid::new_v4();
            let (sender, opened) = oneshot::channel();
            {
                let mut state = self.st();
                if state.is_online {
                    let usable = !state.connection_needs_attention;
                    if let Some(provider) = state
                        .provider
                        .as_ref()
                        .filter(|provider| usable && provider.owns(job))
                    {
                        return Ok(provider.clone());
                    }
                    if is_before_submit(&job.state) {
                        return Err(StepError::Engine(EngineError::NotConnected));
                    }
                }
                state.gate.wait(token, sender);
            }
            self.wait_for_gate(token, opened).await;
        }
    }

    pub(crate) async fn wait_until_online(&self) -> Step {
        loop {
            if cancel::is_cancelled() {
                return Err(StepError::Cancelled);
            }
            let token = Uuid::new_v4();
            let (sender, opened) = oneshot::channel();
            {
                let mut state = self.st();
                if state.is_online {
                    return Ok(());
                }
                state.gate.wait(token, sender);
            }
            self.wait_for_gate(token, opened).await;
        }
    }

    /// Returns when connectivity or the connection changes, or when the task is cancelled.
    async fn wait_for_gate(&self, token: Uuid, opened: oneshot::Receiver<()>) {
        if cancel::cancellable(opened).await.is_err() {
            self.st().gate.open_one(token);
        }
    }

    pub(crate) async fn acquire_slot(&self, id: Uuid) -> Step {
        if self.st().slot_holders.contains(&id) {
            return Ok(());
        }
        let job = self.load_job(id).await?;
        let order = SlotOrder::new(job.created_at, job.promoted_at);
        cancel::cancellable(self.0.limiter.acquire(id, order)).await?;
        self.st().slot_holders.insert(id);
        Ok(())
    }

    pub(crate) fn occupy_slot(&self, id: Uuid) {
        let mut state = self.st();
        if state.slot_holders.insert(id) {
            self.0.limiter.occupy();
        }
    }

    pub(crate) fn release_slot(&self, id: Uuid) {
        if self.st().slot_holders.remove(&id) {
            self.0.limiter.release();
        }
    }

    /// Waits on the engine's clock; a cancelled task stops waiting at once.
    pub(crate) async fn sleep(&self, duration: Duration) -> Step {
        cancel::cancellable(self.0.clock.sleep(duration)).await?;
        Ok(())
    }

    /// Today's spend without `job` and the cap of its connection; `None` skips the check.
    pub(crate) async fn spend_status(&self, job: &Job) -> Step<Option<SpendStatus>> {
        let Some(read) = &self.0.spend_status else {
            return Ok(None);
        };
        Ok(read(job.clone()).await?)
    }
}
