//! Polling the jobs of a connection that reads several statuses in one call (`GenerationProvider::statuses`). Each
//! job's task still drives its own state; instead of sleeping and asking for its status, it waits for the next shared
//! call, which one ticker task makes for every waiting job at once, in groups of `status_batch_limit`.

use std::collections::HashMap;

use jutsu_core::{BatchStatus, Job, PortError};
use time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{JobEngine, State, is_queued_or_processing};
use crate::bookkeeping::{StatusBatch, StatusPace};
use crate::cancel;
use crate::error::{Step, StepError};

pub(crate) use crate::bookkeeping::BatchedStatus;

impl JobEngine {
    /// Whether `job` waits for the shared status call: it was submitted with the active connection, and that one
    /// reads several statuses at once.
    pub(crate) fn polls_in_batch(&self, job: &Job) -> bool {
        polls_in_batch(&self.st(), job)
    }

    /// Waits for the next shared status call and returns what it said about job `id`. A cancelled task stops
    /// waiting at once, even while the call is still out.
    pub(crate) async fn batched_status(&self, id: Uuid) -> Step<BatchedStatus> {
        if cancel::is_cancelled() {
            return Err(StepError::Cancelled);
        }
        let (sender, answer) = oneshot::channel();
        {
            let mut state = self.st();
            state.status_batch.wait(id, sender);
        }
        self.start_status_ticker();
        match cancel::cancellable(answer).await {
            Ok(answer) => Ok(answer.unwrap_or(BatchedStatus::Alone)),
            Err(cancelled) => {
                self.st().status_batch.drop_waiter(id);
                Err(cancelled.into())
            }
        }
    }

    fn start_status_ticker(&self) {
        let mut state = self.st();
        if state.status_batch.ticker.is_some() {
            return;
        }
        let token = CancellationToken::new();
        state.status_batch.ticker = Some(token.clone());
        let engine = self.clone();
        let tick = Box::pin(async move { engine.tick_statuses().await });
        tokio::spawn(cancel::scope(token, tick));
    }

    /// While jobs poll through it: sleeps as long as `StatusPace` says, then answers every waiting job from one call
    /// per group.
    async fn tick_statuses(&self) {
        let mut pace = StatusPace::new(&self.settings(), self.0.random.clone());
        loop {
            if cancel::is_cancelled() || self.sleep(pace.delay).await.is_err() {
                return;
            }
            let waiting = {
                let mut state = self.st();
                if state.status_batch.is_empty() {
                    // Jobs still handling the last answer join the next call; without any, the ticker ends. Checked
                    // and ended in one turn, so a job that starts waiting meanwhile starts a new ticker.
                    if !has_batch_polled_runners(&state) {
                        state.status_batch.ticker = None;
                        return;
                    }
                    continue;
                }
                state.status_batch.waiting()
            };
            let (answers, poll_after) = self.statuses(&waiting).await;
            if cancel::is_cancelled() {
                return;
            }
            self.st().status_batch.answer(&waiting, &answers);
            pace.update(&answers, poll_after);
        }
    }

    /// One call per group of up to `status_batch_limit` jobs, oldest first. Jobs of another connection, groups the
    /// provider couldn't answer and jobs missing from an answer get no entry. `poll_after` is the shortest wait asked.
    async fn statuses(&self, ids: &[Uuid]) -> (HashMap<Uuid, BatchedStatus>, Option<Duration>) {
        let (provider, mut polled) = {
            let state = self.st();
            let Some(provider) = state.provider.clone() else {
                return (HashMap::new(), None);
            };
            let polled: Vec<Job> = ids
                .iter()
                .filter_map(|id| state.jobs.get(id))
                .filter(|job| provider.owns(job) && job.remote.is_some())
                .cloned()
                .collect();
            (provider, polled)
        };
        polled.sort_by(|a, b| {
            (a.created_at, a.id.hyphenated().to_string().to_uppercase())
                .cmp(&(b.created_at, b.id.hyphenated().to_string().to_uppercase()))
        });
        let mut answers = HashMap::new();
        let mut poll_after: Option<Duration> = None;
        for group in StatusBatch::groups(&polled, provider.status_batch_limit()) {
            let remotes: Vec<_> = group.iter().filter_map(|job| job.remote.clone()).collect();
            let batch: Option<BatchStatus> = match cancel::cancellable(provider.statuses(&remotes)).await {
                Err(_) | Ok(Err(PortError::Cancelled)) => break,
                Ok(Ok(batch)) => batch,
                Ok(Err(error)) => {
                    tracing::error!(%error, "shared status call failed, polling each job");
                    None
                }
            };
            let Some(batch) = batch else {
                continue;
            };
            if let Some(asked) = batch.poll_after {
                poll_after = Some(poll_after.map_or(asked, |shortest| shortest.min(asked)));
            }
            for job in &group {
                if let Some(status) = job
                    .remote
                    .as_ref()
                    .and_then(|remote| batch.statuses.get(&remote.request_id))
                {
                    answers.insert(job.id, BatchedStatus::Status(status.clone()));
                }
            }
        }
        (answers, poll_after)
    }
}

fn polls_in_batch(state: &State, job: &Job) -> bool {
    let Some(provider) = &state.provider else {
        return false;
    };
    job.remote.is_some() && provider.status_batch_limit() > 1 && provider.owns(job)
}

/// Whether any job's task polls through the shared call, though it may not be waiting for it right now.
fn has_batch_polled_runners(state: &State) -> bool {
    state.jobs.values().any(|job| {
        is_queued_or_processing(&job.state)
            && state.runners.contains_key(&job.id)
            && polls_in_batch(state, job)
    })
}
