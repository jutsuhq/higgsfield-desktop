//! The spending guard of agents' jobs (`Job::agent`): checked right before every paid submit, never by the agent.

use jutsu_core::{
    AgentHold, AgentMaxCostRaise, AgentMoney, AgentSpend, AgentSpendContext, AgentSpendDecision, Budget,
    BudgetDecision, Cost, Job, JobEvent, JobState, PauseReason, RemoteInputs,
};
use uuid::Uuid;

use super::JobEngine;
use crate::error::{EngineError, Step};

/// What `clear_agent_spend` lets the submit do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentClearance {
    /// Send it; the ledger entry it holds, `None` for a job no agent queued or a submit already reserved.
    Send {
        /// The reservation.
        reservation: Option<Uuid>,
    },
    /// The job paused; send nothing.
    Paused,
}

impl JobEngine {
    /// Runs right before an agent's job is sent, once its slot is taken: the person's own daily cap, strictly, then
    /// the agent's `max_cost`, an unknown price and today's agent budget (`AgentSpendDecision`), and last the
    /// reservation, written to the ledger before anything is sent. A job the plan's unlimited allowance pays for
    /// skips the cap and the budget, which don't count it, but goes out only when the person lets agents use the
    /// allowance (`AgentSpendPolicy::allows_unlimited`); it reserves 0, and the charge the answer reports replaces
    /// that. Once such a job was charged anyway today (`AgentSpendTotal::unlimited_charged`), the allowance is no
    /// longer trusted to pay: until the day ends, agents' jobs for it keep the cap, `max_cost` and the budget like
    /// any other. Such a job skipped the quote before the submit (`price_fits_cap`), so it is quoted here: without
    /// that price its estimate would hold it as `UnknownPrice` however far within `max_cost` it is.
    /// Over any limit, or when the ledger can't be read or written, the job pauses (`PriceAboveCap` or `AgentLimit`)
    /// and only the person resumes it (the agent may raise `max_cost` alone, `raise_agent_max_cost`). A submit that
    /// may already have reached the server is resent with the reservation it holds and no new check.
    pub(crate) async fn clear_agent_spend(&self, id: Uuid, inputs: &RemoteInputs) -> Step<AgentClearance> {
        let mut job = self.load_job(id).await?;
        let Some(origin) = job.agent.clone().filter(|_| job.submit_attempt.is_none()) else {
            return Ok(AgentClearance::Send { reservation: None });
        };
        let Some(policy) = self.0.agent_policy.clone() else {
            return self
                .hold_agent_job(id, AgentHold::LedgerUnavailable, inputs)
                .await;
        };
        let allows_unlimited = (policy.allows_unlimited)();
        let mut charged_today = false;
        if job.spec.uses_unlimited && allows_unlimited {
            match policy.ledger.total(self.now(), origin.max_cost.unit).await {
                Ok(total) => charged_today = total.unlimited_charged,
                Err(error) => {
                    tracing::error!(job = %id, %error, "the agent spend ledger can't be read");
                    return self
                        .hold_agent_job(id, AgentHold::LedgerUnavailable, inputs)
                        .await;
                }
            }
        }
        let unlimited =
            AgentSpendDecision::pays_unlimited(job.spec.uses_unlimited, allows_unlimited, charged_today);
        if !unlimited && job.spec.uses_unlimited {
            job = self.add_server_price(job, inputs).await?;
        }
        if !unlimited
            && let Some(status) = self.spend_status(&job).await?
            && let Some(cap) = status.cap.as_ref()
            && Budget::allows(job.price(), status.committed_today, Some(cap)) != BudgetDecision::Allowed
            && !Budget::covers(job.approved_price.as_ref(), job.price())
        {
            let inputs = inputs.clone();
            self.apply(JobEvent::Paused(PauseReason::PriceAboveCap), id, move |job| {
                job.inputs = Some(inputs);
            })
            .await?;
            return Ok(AgentClearance::Paused);
        }
        let decision = AgentSpendDecision::decide(
            job.price(),
            job.agent_price_is_known(),
            &origin,
            (policy.daily_limit)(job.provider_kind),
            &AgentSpendContext {
                pays_unlimited: unlimited,
                allows_unlimited,
                approved: job.approved_price.clone(),
            },
        );
        match decision {
            AgentSpendDecision::Hold(hold) => self.hold_agent_job(id, hold, inputs).await,
            AgentSpendDecision::Send { money, limit } => {
                let spend = AgentSpend::new(Uuid::new_v4(), id, money, self.now(), unlimited);
                let reservation = spend.id;
                match policy.ledger.reserve(spend, limit).await {
                    Ok(true) => Ok(AgentClearance::Send {
                        reservation: Some(reservation),
                    }),
                    Ok(false) => self.hold_agent_job(id, AgentHold::OverDailyBudget, inputs).await,
                    Err(error) => {
                        tracing::error!(job = %id, %error, "the agent spend ledger can't be written");
                        self.hold_agent_job(id, AgentHold::LedgerUnavailable, inputs)
                            .await
                    }
                }
            }
        }
    }

    /// An agent's own job held because its price is above `max_cost`, sent again by the agent with a higher one:
    /// checked as the guard would check it now, the person's cap first, then the new `max_cost` and what is left of
    /// today's agent budget. When it fits, the job goes on with the new `max_cost` and no approval, so the guard
    /// checks it again in full right before its submit. Holds for any other reason stay for the person.
    pub(crate) async fn raise_max_cost(
        &self,
        id: Uuid,
        max_cost: AgentMoney,
    ) -> Result<AgentMaxCostRaise, EngineError> {
        if !may_raise(&self.load_job(id).await?, max_cost) {
            return Ok(AgentMaxCostRaise::NotLiftable);
        }
        let job = self.load_job(id).await?;
        let (Some(policy), Some(mut origin)) = (self.0.agent_policy.clone(), job.agent.clone()) else {
            return Ok(AgentMaxCostRaise::Held(AgentHold::LedgerUnavailable));
        };
        if let Some(status) = self.spend_status(&job).await?
            && let Some(cap) = status.cap.as_ref()
            && Budget::allows(job.price(), status.committed_today, Some(cap)) != BudgetDecision::Allowed
        {
            return Ok(AgentMaxCostRaise::OverCap);
        }
        let remaining = match policy.ledger.total(self.now(), max_cost.unit).await {
            Ok(total) => total.remaining((policy.daily_limit)(job.provider_kind)),
            Err(error) => {
                tracing::error!(job = %id, %error, "the agent spend ledger can't be read");
                return Ok(AgentMaxCostRaise::Held(AgentHold::LedgerUnavailable));
            }
        };
        origin.max_cost = max_cost;
        let decision = AgentSpendDecision::decide(
            job.price(),
            job.agent_price_is_known(),
            &origin,
            remaining,
            &AgentSpendContext::default(),
        );
        if let AgentSpendDecision::Hold(hold) = decision {
            return Ok(AgentMaxCostRaise::Held(hold));
        }
        // The reads above let the person's Resume or the same raise sent again act on the job meanwhile, and a save
        // of theirs still in flight isn't in `jobs` yet: checked again within the write turn.
        let lifted = self
            .apply_if_still(
                JobEvent::Resume,
                id,
                move |job| may_raise(job, max_cost),
                move |job| {
                    if let Some(agent) = job.agent.as_mut() {
                        agent.max_cost = max_cost;
                        agent.hold = None;
                    }
                },
            )
            .await?;
        if lifted.is_none() {
            return Ok(AgentMaxCostRaise::NotLiftable);
        }
        tracing::info!(job = %id, "the agent raised its max_cost above the price; it goes on");
        self.start_runner(id);
        Ok(AgentMaxCostRaise::Lifted)
    }

    async fn hold_agent_job(&self, id: Uuid, hold: AgentHold, inputs: &RemoteInputs) -> Step<AgentClearance> {
        tracing::info!(job = %id, ?hold, "an agent's job is held");
        let inputs = inputs.clone();
        self.apply(JobEvent::Paused(PauseReason::AgentLimit), id, move |job| {
            job.inputs = Some(inputs);
            if let Some(agent) = job.agent.as_mut() {
                agent.hold = Some(hold);
            }
        })
        .await?;
        Ok(AgentClearance::Paused)
    }

    /// Gives back the reservations a crash orphaned: saved to the ledger, but the app stopped before the job saved
    /// it with its submit marker (`begin_submit`), so nothing was sent. A reservation whose job holds it, or whose
    /// job has a submit marker, may have been sent and stays; so does one whose job isn't in the store (an image
    /// request, reconciled by its own log, or a job the store can't read). Runs at launch, before any job runs.
    pub(crate) async fn reconcile_agent_reservations(&self) {
        let Some(policy) = &self.0.agent_policy else {
            return;
        };
        let reservations = match policy.ledger.unsettled().await {
            Ok(reservations) => reservations,
            Err(error) => {
                tracing::error!(%error, "the agent spend ledger can't be read to reconcile it");
                return;
            }
        };
        for spend in reservations {
            let Ok(Some(job)) = self.0.repository.job(spend.job_id).await else {
                continue;
            };
            if job.agent.as_ref().and_then(|agent| agent.reservation) == Some(spend.id)
                || job.submit_attempt.is_some()
            {
                continue;
            }
            tracing::error!(job = %job.id, "released an agent reservation its submit never took");
            self.release_agent_spend(Some(spend.id), job.id).await;
        }
    }

    /// Gives back a reservation whose submit certainly never reached the provider. A ledger that can't be written
    /// keeps it, which only counts too much.
    pub(crate) async fn release_agent_spend(&self, reservation: Option<Uuid>, id: Uuid) {
        let (Some(reservation), Some(policy)) = (reservation, &self.0.agent_policy) else {
            return;
        };
        if let Err(error) = policy.ledger.release(reservation).await {
            tracing::error!(job = %id, %error, "an agent reservation can't be released");
        }
    }

    /// Keeps counting, at the amount reserved, a reservation the job lets go of: a retry starts a new attempt, and a
    /// new submit reserves anew. A reservation that certainly sent nothing is released and taken off the job when
    /// that is known (`settle_failed_submit`, `begin_submit`), so one the job still holds may have reached the
    /// provider (cut off, unanswered, a submit without an idempotency key) and may have been charged. Settling it
    /// also keeps the launch from giving it back once no job holds it (`reconcile_agent_reservations`). Fails when
    /// the ledger can't be written, so the job keeps holding it.
    pub(crate) async fn keep_agent_spend(&self, reservation: Option<Uuid>, id: Uuid) -> Step {
        let (Some(reservation), Some(policy)) = (reservation, &self.0.agent_policy) else {
            return Ok(());
        };
        if let Err(error) = policy.ledger.settle(reservation, None).await {
            tracing::error!(job = %id, %error, "an agent's earlier charge can't be recorded");
            return Err(error.into());
        }
        Ok(())
    }

    /// Replaces the reservation of an answered submit with what it charged, else keeps the reserved amount.
    pub(crate) async fn settle_agent_spend(&self, job: &Job, charged: Option<&Cost>) {
        let (Some(reservation), Some(policy)) = (
            job.agent.as_ref().and_then(|agent| agent.reservation),
            &self.0.agent_policy,
        ) else {
            return;
        };
        let amount = charged.and_then(AgentMoney::from_cost).map(|money| money.amount);
        if let Err(error) = policy.ledger.settle(reservation, amount).await {
            tracing::error!(job = %job.id, %error, "an agent's charge can't be recorded");
        }
    }

    /// The job ended in a way Higgsfield doesn't bill (its generation failed, moderation refused it, or it was
    /// cancelled before it ran): its share of its submit's ledger entry stops counting, all of it for a submit of
    /// one job. Only called once the provider said so; a job whose outcome isn't known (cut off, past the deadline,
    /// unanswered) never comes here and keeps counting. A ledger that can't be written keeps it, which only counts
    /// too much.
    pub(crate) async fn unbill_agent_spend(&self, id: Uuid) {
        let Some(origin) = self.job(id).and_then(|job| job.agent) else {
            return;
        };
        let (Some(entry), Some(policy)) = (origin.ledger_entry(), &self.0.agent_policy) else {
            return;
        };
        let part = jutsu_core::support::uuid_string(id);
        if let Err(error) = policy
            .ledger
            .unbill(entry, part, origin.paid_parts.unwrap_or(1))
            .await
        {
            tracing::error!(job = %id, %error, "an agent's unbilled job can't be recorded");
        }
    }
}

fn may_raise(job: &Job, max_cost: AgentMoney) -> bool {
    let Some(origin) = &job.agent else {
        return false;
    };
    let raisable = job.state == JobState::Paused(PauseReason::AgentLimit)
        && origin.hold == Some(AgentHold::OverMaxCost)
        && !origin.approved_by_person
        && job.submit_attempt.is_none()
        && job.remote.is_none()
        && !job.spec.uses_unlimited;
    raisable && max_cost.unit == origin.max_cost.unit && max_cost.amount > origin.max_cost.amount
}
