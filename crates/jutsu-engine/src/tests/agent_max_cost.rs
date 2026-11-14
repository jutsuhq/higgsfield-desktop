//! An agent raising `max_cost` on its own job held above it: a port of `AgentMaxCostTests`. The job goes on only
//! when the new `max_cost`, what is left of today's agent budget and the person's cap all fit, and every other hold
//! stays for the person.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jutsu_core::{
    AgentHold, AgentMaxCostRaise, AgentMoney, AgentMoneyUnit, AgentSpend, AgentSpendLedger, Cost, JobState,
    PauseReason,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::agent::{Agents, by_agent, cents, origin};
use super::{make_spec, paused, quiesce, read_spend, spend};
use crate::clock::EngineClock;

fn held_over(max_cents: i64) -> crate::EnqueueOptions {
    by_agent(origin(max_cents, "r-1"))
}

/// The fake's list price is an exact $1.30, 130 cents.
#[tokio::test]
async fn a_max_cost_that_covers_the_price_lifts_the_hold_and_the_job_runs() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let job = engine.enqueue(make_spec(1), held_over(129)).await.unwrap();
    agents.held(job.id).await;

    assert_eq!(
        engine.raise_agent_max_cost(job.id, cents(130)).await.unwrap(),
        AgentMaxCostRaise::Lifted
    );
    let done = agents.done(job.id).await;

    let origin = done.agent.unwrap();
    assert_eq!(origin.max_cost, cents(130));
    assert!(!origin.approved_by_person && origin.hold.is_none());
    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await.spent, 130);
}

#[tokio::test]
async fn a_max_cost_still_below_the_price_changes_nothing() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let job = engine.enqueue(make_spec(1), held_over(100)).await.unwrap();
    agents.held(job.id).await;

    assert_eq!(
        engine.raise_agent_max_cost(job.id, cents(129)).await.unwrap(),
        AgentMaxCostRaise::Held(AgentHold::OverMaxCost)
    );
    quiesce().await;

    let held = agents.stored(job.id);
    let origin = held.agent.unwrap();
    assert!(held.state == paused(PauseReason::AgentLimit) && origin.hold == Some(AgentHold::OverMaxCost));
    assert_eq!(origin.max_cost, cents(100));
    assert_eq!(agents.submits(), 0);
}

/// What reservations and charges took today counts: the new max_cost fits, the rest of the budget doesn't.
#[tokio::test]
async fn a_price_over_what_is_left_of_the_budget_stays_held() {
    let agents = Agents::new();
    let taken = AgentSpend::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        cents(100),
        agents.harness.clock.now(),
        false,
    );
    assert!(agents.ledger.reserve(taken, None).await.unwrap());
    let engine = agents.engine_with(200, false).await;
    let job = engine.enqueue(make_spec(1), held_over(50)).await.unwrap();
    agents.held(job.id).await;

    assert_eq!(
        engine.raise_agent_max_cost(job.id, cents(500)).await.unwrap(),
        AgentMaxCostRaise::Held(AgentHold::OverDailyBudget)
    );
    quiesce().await;

    assert_eq!(agents.stored(job.id).state, paused(PauseReason::AgentLimit));
    assert_eq!(agents.submits(), 0);
}

#[tokio::test]
async fn the_persons_cap_still_applies() {
    let agents = Agents::new();
    let raised = Arc::new(AtomicBool::new(false));
    let mut deps = agents.harness.deps(Some(agents.policy(1_000, false)));
    let read = raised.clone();
    deps.spend_status = Some(read_spend(move || {
        Ok(Some(spend(
            if read.load(Ordering::SeqCst) { "1.5" } else { "0" },
            "2",
        )))
    }));
    let engine = agents.harness.engine_with(deps).await;
    let job = engine.enqueue(make_spec(1), held_over(50)).await.unwrap();
    agents.held(job.id).await;

    raised.store(true, Ordering::SeqCst);
    assert_eq!(
        engine.raise_agent_max_cost(job.id, cents(500)).await.unwrap(),
        AgentMaxCostRaise::OverCap
    );
    quiesce().await;

    assert_eq!(agents.stored(job.id).state, paused(PauseReason::AgentLimit));
    assert_eq!(agents.submits(), 0);
}

/// A price not known, a lower or other-unit max_cost, or a job no longer held: nothing for the agent to lift.
#[tokio::test]
async fn other_holds_and_other_amounts_are_not_liftable() {
    let agents = Agents::new();
    agents
        .harness
        .provider
        .with(|state| state.preliminary = Some(Cost::usd(Decimal::ONE, true)));
    let engine = agents.engine().await;
    let unknown = engine.enqueue(make_spec(1), held_over(500)).await.unwrap();
    agents.held(unknown.id).await;
    assert_eq!(
        agents.stored(unknown.id).agent.unwrap().hold,
        Some(AgentHold::UnknownPrice)
    );

    assert_eq!(
        engine.raise_agent_max_cost(unknown.id, cents(900)).await.unwrap(),
        AgentMaxCostRaise::NotLiftable
    );
    assert_eq!(agents.stored(unknown.id).state, paused(PauseReason::AgentLimit));

    let quoted = Agents::new();
    let other = quoted.engine().await;
    let job = other.enqueue(make_spec(1), held_over(100)).await.unwrap();
    quoted.held(job.id).await;
    assert_eq!(
        other.raise_agent_max_cost(job.id, cents(90)).await.unwrap(),
        AgentMaxCostRaise::NotLiftable
    );
    assert_eq!(
        other.raise_agent_max_cost(job.id, cents(100)).await.unwrap(),
        AgentMaxCostRaise::NotLiftable
    );
    assert_eq!(
        other
            .raise_agent_max_cost(job.id, AgentMoney::new(500, AgentMoneyUnit::Credits))
            .await
            .unwrap(),
        AgentMaxCostRaise::NotLiftable
    );

    other.resume(job.id).await.unwrap();
    quoted.done(job.id).await;
    assert_eq!(
        other.raise_agent_max_cost(job.id, cents(500)).await.unwrap(),
        AgentMaxCostRaise::NotLiftable
    );
    assert_eq!(quoted.submits(), 1);
}

/// The raise checks the budget as it is then; the guard checks it again right before the submit, so a job that took
/// the rest of the budget meanwhile holds the lifted job again, with nothing sent.
#[tokio::test]
async fn a_lifted_job_is_held_again_when_the_budget_is_taken_before_its_submit() {
    let agents = Agents::new();
    let engine = agents.engine_with(200, false).await;
    let job = engine.enqueue(make_spec(1), held_over(129)).await.unwrap();
    agents.held(job.id).await;
    let (reached, release) = agents
        .harness
        .repository
        .hold_save(|job| job.state == JobState::Preparing);

    let raise = tokio::spawn({
        let engine = engine.clone();
        async move { engine.raise_agent_max_cost(job.id, cents(130)).await }
    });
    reached.wait().await;
    let other = AgentSpend::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        cents(100),
        agents.harness.clock.now(),
        false,
    );
    assert!(agents.ledger.reserve(other, Some(200)).await.unwrap());
    release.open();
    assert_eq!(raise.await.unwrap().unwrap(), AgentMaxCostRaise::Lifted);

    agents
        .harness
        .repository
        .wait(job.id, |job| {
            job.agent.as_ref().and_then(|agent| agent.hold) == Some(AgentHold::OverDailyBudget)
        })
        .await;
    let held = agents.stored(job.id);
    assert!(held.state == paused(PauseReason::AgentLimit) && held.agent.unwrap().max_cost == cents(130));
    assert_eq!(agents.submits(), 0);
    assert_eq!(agents.total().await.taken(), 100);
}

/// The same raise sent again while the first one's save is still on its way: the second sees the lifted job only
/// within its write turn, and answers not liftable instead of resuming a job that already goes on.
#[tokio::test]
async fn a_raise_sent_again_while_the_first_is_saved_lifts_once() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let job = engine.enqueue(make_spec(1), held_over(129)).await.unwrap();
    agents.held(job.id).await;
    let (reached, release) = agents
        .harness
        .repository
        .hold_save(|job| job.state == JobState::Preparing);

    let raise =
        |engine: crate::JobEngine| async move { engine.raise_agent_max_cost(job.id, cents(130)).await };
    let first = tokio::spawn(raise(engine.clone()));
    reached.wait().await;
    let second = tokio::spawn(raise(engine.clone()));
    // The second has to wait for the write turn while the first is held; nothing observable marks that.
    quiesce().await;
    release.open();

    let mut answers = [first.await.unwrap().unwrap(), second.await.unwrap().unwrap()];
    answers.sort_by_key(|answer| format!("{answer:?}"));
    assert_eq!(
        answers,
        [AgentMaxCostRaise::Lifted, AgentMaxCostRaise::NotLiftable]
    );
    let done = agents.done(job.id).await;
    let origin = done.agent.unwrap();
    assert!(origin.max_cost == cents(130) && !origin.approved_by_person);
    assert_eq!(agents.submits(), 1);
    assert_eq!(agents.total().await.spent, 130);
}

/// The person's Confirm while the agent's raise is being saved: the job goes on once, under the agent's raise.
#[tokio::test]
async fn the_persons_resume_during_a_raise_leaves_the_lifted_job() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let job = engine.enqueue(make_spec(1), held_over(129)).await.unwrap();
    agents.held(job.id).await;
    let (reached, release) = agents
        .harness
        .repository
        .hold_save(|job| job.state == JobState::Preparing);

    let raise = tokio::spawn({
        let engine = engine.clone();
        async move { engine.raise_agent_max_cost(job.id, cents(130)).await }
    });
    reached.wait().await;
    let resume = tokio::spawn({
        let engine = engine.clone();
        async move { engine.resume(job.id).await }
    });
    quiesce().await;
    release.open();

    assert_eq!(raise.await.unwrap().unwrap(), AgentMaxCostRaise::Lifted);
    resume.await.unwrap().unwrap();
    let done = agents.done(job.id).await;
    assert!(!done.agent.unwrap().approved_by_person);
    assert_eq!(agents.submits(), 1);
}

/// The agent's raise while the person's Confirm is being saved: the person decided, so nothing is lifted.
#[tokio::test]
async fn a_raise_during_the_persons_resume_is_not_liftable() {
    let agents = Agents::new();
    let engine = agents.engine().await;
    let job = engine.enqueue(make_spec(1), held_over(129)).await.unwrap();
    agents.held(job.id).await;
    let (reached, release) = agents
        .harness
        .repository
        .hold_save(|job| job.state == JobState::Preparing);

    let resume = tokio::spawn({
        let engine = engine.clone();
        async move { engine.resume(job.id).await }
    });
    reached.wait().await;
    let raise = tokio::spawn({
        let engine = engine.clone();
        async move { engine.raise_agent_max_cost(job.id, cents(130)).await }
    });
    quiesce().await;
    release.open();

    resume.await.unwrap().unwrap();
    assert_eq!(raise.await.unwrap().unwrap(), AgentMaxCostRaise::NotLiftable);
    let done = agents.done(job.id).await;
    let origin = done.agent.unwrap();
    assert!(origin.approved_by_person && origin.max_cost == cents(129));
    assert_eq!(agents.submits(), 1);
}
