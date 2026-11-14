//! Queueing jobs: one at a time, each after the one before is saved.

use futures::FutureExt;
use futures::future::BoxFuture;
use jutsu_core::{AgentOrigin, AspectRatio, Budget, BudgetDecision, Job, JobSpec};
use uuid::Uuid;

use super::{JobEngine, SharedJob};
use crate::error::{EngineError, OverCap};

/// What makes two enqueues the same request. The batch isn't part of it: each submit of a batch makes a new one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnqueueRequest {
    spec: JobSpec,
    parent_id: Option<Uuid>,
    /// Two agent requests of the same spec are two jobs.
    agent_request: Option<String>,
}

/// How a job is queued.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnqueueOptions {
    /// The library item the job is made from, if any.
    pub parent_id: Option<Uuid>,
    /// The batch the job belongs to; batch jobs take slots like any other job.
    pub batch_id: Option<Uuid>,
    /// The user agreed to go over today's cap with this job.
    pub over_cap_approved: bool,
    /// The job's ID, when the caller recorded it before queueing (an agent's request); a job with that ID already
    /// queued is returned as it is.
    pub id: Option<Uuid>,
    /// The agent that queued it; such a job never passes the cap on its own (`over_cap_approved` is ignored) and is
    /// checked by the spending guard before its submit (`clear_agent_spend`).
    pub agent: Option<AgentOrigin>,
}

/// Takes a finished enqueue off the engine's list, whatever way its caller stops waiting.
struct EnqueueFinish {
    engine: JobEngine,
    token: Uuid,
}

impl Drop for EnqueueFinish {
    fn drop(&mut self) {
        self.engine
            .st()
            .enqueuing
            .retain(|(token, _, _)| *token != self.token);
    }
}

impl JobEngine {
    /// Queues a job. The caller asks the user first when `Budget::allows` says so; the engine checks the cap again
    /// (`spend_status`) and fails with `OverCap` unless `over_cap_approved`, so two submits that both read today's
    /// spend before either was queued can't both pass it. Enqueues run one at a time, each after the one before is
    /// saved. An enqueue of the same spec and parent while an equal one is still running (a double click) returns
    /// that job instead of queueing and paying for a second.
    pub(crate) async fn enqueue_job(
        &self,
        spec: JobSpec,
        options: EnqueueOptions,
    ) -> Result<Job, EngineError> {
        let request = EnqueueRequest {
            spec: spec.clone(),
            parent_id: options.parent_id,
            agent_request: options.agent.as_ref().map(|agent| agent.request_id.clone()),
        };
        let token = Uuid::new_v4();
        // An equal enqueue still running gives its job; only the enqueue that started a task takes it off the list.
        let task = 'started: {
            let mut state = self.st();
            if let Some((_, _, running)) = state.enqueuing.iter().find(|(_, other, _)| *other == request) {
                break 'started running.clone();
            }
            let previous = state.last_enqueue.take();
            let approved = options.over_cap_approved && options.agent.is_none();
            let engine = self.clone();
            let insert: BoxFuture<'static, Result<Job, EngineError>> = Box::pin(async move {
                if let Some(previous) = previous {
                    previous.await;
                }
                engine.insert(spec, options, approved).await
            });
            // Runs on its own, as a Swift `Task`: a caller that stops waiting doesn't stop the enqueue.
            let handle = tokio::spawn(insert);
            let task: SharedJob = async move { handle.await.unwrap_or(Err(EngineError::Cancelled)) }
                .boxed()
                .shared();
            let done = task.clone();
            state.last_enqueue = Some(
                async move {
                    let _ = done.await;
                }
                .boxed()
                .shared(),
            );
            state.enqueuing.push((token, request, task.clone()));
            task
        };
        let _finish = EnqueueFinish {
            engine: self.clone(),
            token,
        };
        task.await
    }

    /// A clip made from a start frame takes the frame's shape as its `aspect_ratio`, the nearest fixed one: the
    /// account's Seedance doesn't derive it and makes 16:9 without one (REST's `image-to-video` does, and ignores it).
    /// A frame that can't be read leaves the server's default.
    /// The provider decides whether the job is paid from the unlimited allowance, not the caller: a spec copied from
    /// an earlier job may carry a choice that no longer holds. The cap doesn't count such a job. Such a job makes one
    /// variant, as the account caps it to; a job of several variants is a batch of its own unless it is part of one,
    /// so the jobs its submit starts are shown together.
    async fn insert(
        &self,
        mut spec: JobSpec,
        options: EnqueueOptions,
        over_cap_approved: bool,
    ) -> Result<Job, EngineError> {
        if let Some(id) = options.id
            && let Ok(known) = self.load_job(id).await
        {
            return Ok(known);
        }
        let Some(provider) = self.st().provider.clone() else {
            return Err(EngineError::NotConnected);
        };
        if spec.uses_frames()
            && spec.aspect_ratio.is_none_or(|ratio| ratio == AspectRatio::Auto)
            && let Some(start) = spec.references.first()
        {
            spec.aspect_ratio = self
                .0
                .media
                .image_size(start)
                .await
                .and_then(|size| AspectRatio::nearest(size, &AspectRatio::ALL));
        }
        spec.uses_unlimited = provider.pays_unlimited(&spec).await;
        if spec.uses_unlimited {
            spec.variants = 1;
        }
        let cost = provider.estimate(&spec).await?;
        let batch_id = options
            .batch_id
            .or_else(|| (spec.variants > 1).then(Uuid::new_v4));
        let uses_unlimited = spec.uses_unlimited;
        let mut job = Job::new(
            options.id.unwrap_or_else(Uuid::new_v4),
            spec,
            provider.kind(),
            cost.clone(),
            Uuid::new_v4(),
            self.now(),
        );
        job.owner = provider.owner();
        job.parent_id = options.parent_id;
        job.batch_id = batch_id;
        job.agent = options.agent;
        // What the user agreed to: a server price above it asks them again.
        if over_cap_approved {
            job.approved_price = Some(cost.clone());
        }
        if !uses_unlimited
            && let Some(status) = self.spend_status(&job).await?
            && !over_cap_approved
            && let Some(cap) = status.cap
            && Budget::allows(&cost, status.committed_today, Some(&cap)) != BudgetDecision::Allowed
        {
            return Err(EngineError::OverCap(OverCap {
                cap,
                committed_today: status.committed_today,
                price: cost,
            }));
        }
        self.0.repository.save(&job).await?;
        self.st().jobs.insert(job.id, job.clone());
        self.publish(&job);
        self.start_runner(job.id);
        Ok(job)
    }
}
