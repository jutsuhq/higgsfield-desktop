//! What makes an agent's paid request (`create_video`, `create_image`, `enhance`) run once for its `request_id`
//! (`AgentRequestGate.swift`): the same request sent while it runs waits for it, one recorded in `AgentRequestLog`
//! answers with what it made, and the ID sent with other parameters is refused. A request is recorded by the run
//! itself (`record`), with the IDs of what it makes, before anything is queued or sent, and forgotten when it fails,
//! so the agent can fix it and send it again.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use time::OffsetDateTime;
use uuid::Uuid;

use super::request_log::{AgentLookup, AgentRequestLog, AgentRequestRecord};
use crate::images::LogWriteError;

/// What a request is, by its ID and fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRequestState {
    /// Never seen: run it.
    New,
    /// The same request is running: it waits for that one.
    Running,
    /// The same request again: these are what it made.
    Known(Vec<Uuid>),
    /// The same parameters with another `max_cost`: what it made, whose hold the new one may lift.
    OtherMaxCost(Vec<Uuid>),
    /// The ID was used with other parameters.
    Conflict,
}

/// The failures the gate answers with instead of running a request.
#[derive(Debug, Clone)]
pub struct Refusals<E> {
    /// The ID was used with other parameters.
    pub conflict: E,
    /// The log can't be read, so whether the request ran isn't known.
    pub unreadable: E,
    /// The request was recorded, but what it makes was never saved: nothing was queued or paid, so it is forgotten
    /// and may be sent again.
    pub never_queued: E,
}

/// What a request sent again with another `max_cost` made of what it recorded (`AgentRequestGate::run`'s
/// `other_max_cost`).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRaiseOutcome<A> {
    /// The answer for the job: lifted, still held, or already raised to this `max_cost`.
    Answer(A),
    /// Nothing the new `max_cost` can lift: refused as a conflict.
    NotLiftable,
    /// The job it recorded isn't there: answered as never queued, and the request forgotten.
    Gone,
}

impl<A> AgentRaiseOutcome<A> {
    pub fn map<B>(self, transform: impl FnOnce(A) -> B) -> AgentRaiseOutcome<B> {
        match self {
            Self::Answer(answer) => AgentRaiseOutcome::Answer(transform(answer)),
            Self::NotLiftable => AgentRaiseOutcome::NotLiftable,
            Self::Gone => AgentRaiseOutcome::Gone,
        }
    }
}

type Run<A, E> = Shared<BoxFuture<'static, Result<A, E>>>;
/// The fingerprint of the request running under an ID, and its run.
type Running<A, E> = HashMap<String, (String, Run<A, E>)>;

/// Runs each paid request once for its request ID.
pub struct AgentRequestGate<A, E> {
    log: Mutex<AgentRequestLog>,
    running: Mutex<Running<A, E>>,
    /// Raises of `max_cost` in flight (`OtherMaxCost`), by request ID and the fingerprint of the new `max_cost`.
    raising: Mutex<HashMap<(String, String), Run<A, E>>>,
}

impl<A, E> std::fmt::Debug for AgentRequestGate<A, E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentRequestGate").finish_non_exhaustive()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<A, E> AgentRequestGate<A, E>
where
    A: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    pub fn new(log: AgentRequestLog) -> Arc<Self> {
        Arc::new(Self {
            log: Mutex::new(log),
            running: Mutex::new(HashMap::new()),
            raising: Mutex::new(HashMap::new()),
        })
    }

    /// The gate over `agent-requests.json` in `folder`; `None` keeps it in memory.
    pub fn open(folder: Option<&Path>) -> Arc<Self> {
        Self::new(AgentRequestLog::new(folder))
    }

    pub fn state(
        &self,
        request_id: &str,
        fingerprint: &str,
        parameters: Option<&str>,
    ) -> Result<AgentRequestState, LogWriteError> {
        if let Some((running, _)) = locked(&self.running).get(request_id) {
            return Ok(if running == fingerprint {
                AgentRequestState::Running
            } else {
                AgentRequestState::Conflict
            });
        }
        Ok(
            match locked(&self.log).lookup(request_id, fingerprint, parameters)? {
                AgentLookup::New => AgentRequestState::New,
                AgentLookup::Known(ids) => AgentRequestState::Known(ids),
                AgentLookup::OtherMaxCost(ids) => AgentRequestState::OtherMaxCost(ids),
                AgentLookup::Conflict => AgentRequestState::Conflict,
            },
        )
    }

    /// Runs `queue` for a new request and answers with what it made. The same request running answers with what that
    /// run makes; one recorded answers with `known` for the IDs it recorded, or `refusals.never_queued` when `known`
    /// finds none of them. One recorded with the same `parameters` and another `max_cost` answers with what
    /// `other_max_cost` makes of what it recorded, `refusals.conflict` when it isn't liftable or not given, and
    /// `refusals.never_queued` (forgotten, as for `known`) when its job isn't there; the same request sent again
    /// while that runs joins it. `discard` is called once the request's own files aren't needed: refused, already
    /// known, joining the run in flight, whose own files are the ones it reads, or failed. A run that fails is
    /// forgotten. The run goes on in a task of its own, whatever becomes of the caller.
    #[allow(clippy::too_many_arguments, reason = "the Swift gate's parameters")]
    pub async fn run<K, KF, O, OF, Q, QF>(
        self: &Arc<Self>,
        request_id: &str,
        fingerprint: &str,
        parameters: Option<&str>,
        refusals: Refusals<E>,
        discard: impl FnOnce(),
        known: K,
        other_max_cost: Option<O>,
        queue: Q,
    ) -> Result<A, E>
    where
        K: FnOnce(Vec<Uuid>) -> KF,
        KF: Future<Output = Result<Option<A>, E>>,
        O: FnOnce(Vec<Uuid>) -> OF,
        OF: Future<Output = Result<AgentRaiseOutcome<A>, E>> + Send + 'static,
        Q: FnOnce() -> QF,
        QF: Future<Output = Result<A, E>> + Send + 'static,
    {
        let running = locked(&self.running).get(request_id).cloned();
        if let Some((running, run)) = running {
            discard();
            if running != fingerprint {
                return Err(refusals.conflict);
            }
            return run.await;
        }
        let raise = (request_id.to_owned(), fingerprint.to_owned());
        let raising = locked(&self.raising).get(&raise).cloned();
        if let Some(run) = raising {
            discard();
            return run.await;
        }
        let lookup = locked(&self.log).lookup(request_id, fingerprint, parameters);
        let lookup = match lookup {
            Ok(lookup) => lookup,
            Err(error) => {
                tracing::error!(%error, "the agent requests can't be read");
                return Err(refusals.unreadable);
            }
        };
        match lookup {
            AgentLookup::Conflict => {
                discard();
                return Err(refusals.conflict);
            }
            AgentLookup::Known(ids) => {
                discard();
                if let Some(answer) = known(ids).await? {
                    return Ok(answer);
                }
                self.forget(request_id);
                return Err(refusals.never_queued);
            }
            AgentLookup::OtherMaxCost(ids) => {
                discard();
                let Some(other_max_cost) = other_max_cost else {
                    return Err(refusals.conflict);
                };
                return self.raise(raise, ids, refusals, other_max_cost).await;
            }
            AgentLookup::New => {}
        }
        let work = queue();
        let gate = Arc::clone(self);
        let id = request_id.to_owned();
        let never_queued = refusals.never_queued.clone();
        let task = tokio::spawn(async move {
            let result = work.await;
            if result.is_err() {
                gate.forget(&id);
            }
            locked(&gate.running).remove(&id);
            result
        });
        let run: Run<A, E> = async move { task.await.unwrap_or(Err(never_queued)) }
            .boxed()
            .shared();
        locked(&self.running).insert(request_id.to_owned(), (fingerprint.to_owned(), run.clone()));
        let result = run.await;
        locked(&self.running).remove(request_id);
        if result.is_err() {
            discard();
        }
        result
    }

    /// Runs `other_max_cost` for the request sent again with another `max_cost`, so the same one sent meanwhile joins
    /// it instead of checking the job a second time.
    async fn raise<O, OF>(
        self: &Arc<Self>,
        raise: (String, String),
        ids: Vec<Uuid>,
        refusals: Refusals<E>,
        other_max_cost: O,
    ) -> Result<A, E>
    where
        O: FnOnce(Vec<Uuid>) -> OF,
        OF: Future<Output = Result<AgentRaiseOutcome<A>, E>> + Send + 'static,
    {
        let work = other_max_cost(ids);
        let gate = Arc::clone(self);
        let key = raise.clone();
        let never_queued = refusals.never_queued.clone();
        let task = tokio::spawn(async move {
            let result = match work.await {
                Ok(AgentRaiseOutcome::Answer(answer)) => Ok(answer),
                Ok(AgentRaiseOutcome::NotLiftable) => Err(refusals.conflict),
                Ok(AgentRaiseOutcome::Gone) => {
                    gate.forget(&key.0);
                    Err(refusals.never_queued)
                }
                Err(error) => Err(error),
            };
            locked(&gate.raising).remove(&key);
            result
        });
        let run: Run<A, E> = async move { task.await.unwrap_or(Err(never_queued)) }
            .boxed()
            .shared();
        locked(&self.raising).insert(raise.clone(), run.clone());
        let result = run.await;
        locked(&self.raising).remove(&raise);
        result
    }

    /// Records the request with the IDs of what it makes, before anything is queued or sent. `kind` is what the IDs
    /// are when the app keeps them only in memory (`AgentRequestRecord::kind`).
    pub fn record(
        &self,
        request_id: &str,
        fingerprint: &str,
        parameters: Option<&str>,
        ids: Vec<Uuid>,
        at: OffsetDateTime,
        kind: Option<&str>,
    ) -> Result<(), LogWriteError> {
        locked(&self.log).add(AgentRequestRecord {
            request_id: request_id.to_owned(),
            fingerprint: fingerprint.to_owned(),
            parameters: parameters.map(str::to_owned),
            job_ids: ids,
            created_at: at,
            kind: kind.map(str::to_owned),
        })
    }

    /// The `kind` the request that made `id` was recorded with.
    pub fn kind_of(&self, id: Uuid) -> Option<String> {
        locked(&self.log).kind_of(id)
    }

    /// The request sent again with another `max_cost` lifted its job's hold: from now on that one is the request.
    pub fn update(&self, request_id: &str, fingerprint: &str) -> Result<(), LogWriteError> {
        locked(&self.log).update(request_id, fingerprint)
    }

    /// Forgets a request that queued nothing, so the agent can send it again. A log that can't be written keeps it,
    /// and the request sent again is then answered as never queued, and forgotten again.
    pub fn forget(&self, request_id: &str) {
        if let Err(error) = locked(&self.log).remove(request_id) {
            tracing::error!(%error, "an agent request can't be forgotten");
        }
    }
}

#[cfg(test)]
#[path = "request_gate_tests.rs"]
mod tests;
