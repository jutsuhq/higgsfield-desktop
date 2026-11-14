//! `create_video` sent again with its parameters and a higher `max_cost` (`AgentRequestState::OtherMaxCost`): what
//! the engine's check of its job (`JobEngine::raise_agent_max_cost`) answers the agent (`AgentMaxCostRaising.swift`).
//! Only the engine decides whether the job goes on; this only words the answer and keeps the request's record in step
//! with it.

use std::fmt::Display;
use std::future::Future;

use jutsu_core::{AgentMaxCostRaise, AgentMoney, Job};
use uuid::Uuid;

use super::job::AgentJobRules;
use super::request_gate::AgentRaiseOutcome;

/// A job of a request sent again with a higher `max_cost`, and why it still waits when it does.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRaisedJob {
    pub job: Job,
    /// Why the job still waits (`AgentJobRules::still_held_note`); `None` once it goes on.
    pub note: Option<String>,
}

/// - `ids`: what the request recorded; only a request of one job can be raised.
/// - `raise`: the engine's check of the job against `max_cost`; `None` when the job isn't there.
/// - `job`: the job as it is now; `None` when it isn't there, and an error when that can't be known (a job not read
///   could still exist, and forgetting its request would let the agent pay twice).
/// - `record_raise`: from now on the request is the one with the new `max_cost` (`AgentRequestGate::update`). A
///   failure is logged: the same request sent again finds the job already raised and records it then.
pub async fn answer<E, R, RF, J, JF, W>(
    ids: &[Uuid],
    max_cost: AgentMoney,
    raise: R,
    job: J,
    record_raise: impl FnOnce() -> Result<(), W>,
) -> Result<AgentRaiseOutcome<AgentRaisedJob>, E>
where
    R: FnOnce(Uuid, AgentMoney) -> RF,
    RF: Future<Output = Result<Option<AgentMaxCostRaise>, E>>,
    J: FnOnce(Uuid) -> JF,
    JF: Future<Output = Result<Option<Job>, E>>,
    W: Display,
{
    let [id] = ids else {
        return Ok(AgentRaiseOutcome::NotLiftable);
    };
    let Some(result) = raise(*id, max_cost).await? else {
        return Ok(AgentRaiseOutcome::Gone);
    };
    let note = match result {
        AgentMaxCostRaise::Lifted => {
            record(record_raise);
            None
        }
        AgentMaxCostRaise::Held(hold) => Some(AgentJobRules::still_held_note(Some(hold))),
        AgentMaxCostRaise::OverCap => Some(AgentJobRules::still_held_note(None)),
        AgentMaxCostRaise::NotLiftable => {
            // Already raised to this max_cost: a raise whose record wasn't updated, or one that finished while this
            // request was on its way. It is the same request, so it answers as known instead of a conflict, which an
            // agent may answer with a new request_id and pay twice.
            let Some(current) = job(*id).await? else {
                return Ok(AgentRaiseOutcome::Gone);
            };
            if current.agent.as_ref().map(|agent| agent.max_cost) != Some(max_cost) {
                return Ok(AgentRaiseOutcome::NotLiftable);
            }
            record(record_raise);
            return Ok(AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: current,
                note: None,
            }));
        }
    };
    let Some(current) = job(*id).await? else {
        return Ok(AgentRaiseOutcome::Gone);
    };
    Ok(AgentRaiseOutcome::Answer(AgentRaisedJob {
        job: current,
        note: note.map(str::to_owned),
    }))
}

fn record<W: Display>(record_raise: impl FnOnce() -> Result<(), W>) {
    if let Err(error) = record_raise() {
        tracing::error!(%error, "the raised max_cost can't be recorded");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use jutsu_core::{AgentHold, AgentMoneyUnit, AgentOrigin, JobState, PauseReason};

    use super::*;
    use crate::queue::fixture;

    #[derive(Debug, PartialEq)]
    struct Failed;

    fn job(max_cents: i64, state: JobState) -> Job {
        let mut job = fixture::JobFixture {
            cost: fixture::usd("1.3"),
            ..fixture::JobFixture::default()
        }
        .job(state);
        job.agent = Some(AgentOrigin::new(
            Some("claude-code".to_owned()),
            "r1",
            AgentMoney::new(max_cents, AgentMoneyUnit::UsdCents),
        ));
        job
    }

    fn held(max_cents: i64) -> Job {
        job(max_cents, JobState::Paused(PauseReason::AgentLimit))
    }

    /// Answers for `ids` as the engine's check says `result`, with `current` as the job now, raising to 130 cents;
    /// counts the raises and the record updates; `None` for `result` is a job the engine doesn't have.
    async fn answer_for(
        ids: &[Uuid],
        result: Option<AgentMaxCostRaise>,
        current: Option<Job>,
        raises: &Cell<usize>,
        records: &Cell<usize>,
        record_fails: bool,
    ) -> AgentRaiseOutcome<AgentRaisedJob> {
        let answered: Result<_, Failed> = answer(
            ids,
            AgentMoney::new(130, AgentMoneyUnit::UsdCents),
            |_, _| {
                raises.set(raises.get() + 1);
                async move { Ok(result) }
            },
            |_| async move { Ok(current) },
            || {
                records.set(records.get() + 1);
                if record_fails { Err("full disk") } else { Ok(()) }
            },
        )
        .await;
        answered.unwrap()
    }

    #[tokio::test]
    async fn a_lifted_job_goes_on_and_the_request_stands_for_the_new_max_cost() {
        let lifted = job(130, JobState::Preparing);
        let records = Cell::new(0);
        let outcome = answer_for(
            &[lifted.id],
            Some(AgentMaxCostRaise::Lifted),
            Some(lifted.clone()),
            &Cell::new(0),
            &records,
            false,
        )
        .await;
        assert_eq!(
            outcome,
            AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: lifted,
                note: None
            })
        );
        assert_eq!(records.get(), 1);
    }

    /// The job goes on even when the request's record can't be updated: the same request sent again finds the job
    /// already raised and records it then.
    #[tokio::test]
    async fn a_record_that_cant_be_updated_still_answers_with_the_lifted_job() {
        let lifted = job(130, JobState::Preparing);
        let records = Cell::new(0);
        let outcome = answer_for(
            &[lifted.id],
            Some(AgentMaxCostRaise::Lifted),
            Some(lifted.clone()),
            &Cell::new(0),
            &records,
            true,
        )
        .await;
        assert_eq!(
            outcome,
            AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: lifted,
                note: None
            })
        );
        assert_eq!(records.get(), 1);
    }

    /// Only a request of one job can be raised: variants, or a record without jobs, are a conflict, unchecked.
    #[tokio::test]
    async fn a_request_of_other_than_one_job_is_not_liftable() {
        let raises = Cell::new(0);
        let held = held(100);
        for ids in [vec![], vec![held.id, Uuid::new_v4()]] {
            let outcome = answer_for(
                &ids,
                Some(AgentMaxCostRaise::Lifted),
                Some(held.clone()),
                &raises,
                &Cell::new(0),
                false,
            )
            .await;
            assert_eq!(outcome, AgentRaiseOutcome::NotLiftable);
        }
        assert_eq!(raises.get(), 0);
    }

    /// Still held, the job comes back with why, and the request keeps its old `max_cost`.
    #[tokio::test]
    async fn a_job_still_held_says_why() {
        let held = held(100);
        let records = Cell::new(0);
        let budget = answer_for(
            &[held.id],
            Some(AgentMaxCostRaise::Held(AgentHold::OverDailyBudget)),
            Some(held.clone()),
            &Cell::new(0),
            &records,
            false,
        )
        .await;
        assert_eq!(
            budget,
            AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: held.clone(),
                note: Some(AgentJobRules::still_held_note(Some(AgentHold::OverDailyBudget)).to_owned()),
            })
        );
        let cap = answer_for(
            &[held.id],
            Some(AgentMaxCostRaise::OverCap),
            Some(held.clone()),
            &Cell::new(0),
            &records,
            false,
        )
        .await;
        assert_eq!(
            cap,
            AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: held,
                note: Some(AgentJobRules::still_held_note(None).to_owned()),
            })
        );
        assert_eq!(records.get(), 0);
    }

    /// A raise that lifted the job but couldn't record it, or one that finished while this request was on its way,
    /// leaves the job running at this `max_cost`: the same request again is known, and recorded now, not a conflict.
    #[tokio::test]
    async fn a_job_already_raised_to_this_max_cost_answers_as_known() {
        let raised = job(130, JobState::Processing);
        let records = Cell::new(0);
        let outcome = answer_for(
            &[raised.id],
            Some(AgentMaxCostRaise::NotLiftable),
            Some(raised.clone()),
            &Cell::new(0),
            &records,
            false,
        )
        .await;
        assert_eq!(
            outcome,
            AgentRaiseOutcome::Answer(AgentRaisedJob {
                job: raised,
                note: None
            })
        );
        assert_eq!(records.get(), 1);
    }

    /// A recorded job that isn't there any more is answered as never queued, as a known request's is.
    #[tokio::test]
    async fn a_job_that_is_gone_is_answered_as_never_queued() {
        let held = held(100);
        let none = answer_for(&[held.id], None, None, &Cell::new(0), &Cell::new(0), false).await;
        assert_eq!(none, AgentRaiseOutcome::Gone);
        for result in [
            AgentMaxCostRaise::Lifted,
            AgentMaxCostRaise::NotLiftable,
            AgentMaxCostRaise::Held(AgentHold::OverMaxCost),
        ] {
            let outcome = answer_for(
                &[held.id],
                Some(result),
                None,
                &Cell::new(0),
                &Cell::new(0),
                false,
            )
            .await;
            assert_eq!(outcome, AgentRaiseOutcome::Gone);
        }
    }

    /// A store that can't be read says nothing about the job: the failure goes to the agent, and the request stays.
    #[tokio::test]
    async fn a_job_that_cant_be_read_is_a_failure_not_gone() {
        let held = held(100);
        let outcome = answer(
            &[held.id],
            AgentMoney::new(130, AgentMoneyUnit::UsdCents),
            |_, _| async { Ok(Some(AgentMaxCostRaise::Lifted)) },
            |_| async { Err(Failed) },
            || Ok::<(), &str>(()),
        )
        .await;
        assert_eq!(outcome, Err(Failed));
    }

    #[tokio::test]
    async fn a_job_not_raised_to_this_max_cost_is_not_liftable() {
        let records = Cell::new(0);
        let mut credits = job(130, JobState::Processing);
        if let Some(agent) = credits.agent.as_mut() {
            agent.max_cost = AgentMoney::new(130, AgentMoneyUnit::Credits);
        }
        for other in [
            job(129, JobState::Processing),
            job(200, JobState::Processing),
            credits,
        ] {
            let outcome = answer_for(
                &[other.id],
                Some(AgentMaxCostRaise::NotLiftable),
                Some(other.clone()),
                &Cell::new(0),
                &records,
                false,
            )
            .await;
            assert_eq!(outcome, AgentRaiseOutcome::NotLiftable);
        }
        assert_eq!(records.get(), 0);
    }
}
