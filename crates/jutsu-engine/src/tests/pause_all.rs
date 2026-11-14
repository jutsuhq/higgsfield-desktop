//! Pause all and Resume all, the person's and an agent's: a port of `PauseAllTests`.

use std::sync::Arc;

use jutsu_core::{AgentOrigin, JobState, PauseReason, PortError, ProviderError, ProviderKind, RemoteStatus};
use time::Duration;
use uuid::Uuid;

use super::agent::cents;
use super::{CallLog, FakeProvider, Harness, SubmitStep, eventually, make_spec, paused, quiesce};
use crate::{EnqueueOptions, JobEngine};

fn holding(max_concurrency: usize) -> Harness {
    let mut harness = Harness::new();
    harness.max_concurrency = max_concurrency;
    harness
        .provider
        .with(|state| state.held_status = Some(RemoteStatus::Queued));
    harness
}

fn count_in(harness: &Harness, state: &JobState) -> usize {
    harness
        .repository
        .all()
        .iter()
        .filter(|job| job.state == *state)
        .count()
}

async fn waiting_for_slot(harness: &Harness, count: usize) {
    harness
        .repository
        .wait_until(|_| count_in(harness, &JobState::WaitingForSlot) == count)
        .await;
}

fn state(harness: &Harness, id: Uuid) -> JobState {
    harness.repository.stored(id).unwrap().state
}

fn by_user() -> JobState {
    paused(PauseReason::ByUser)
}

/// One job running, then one of the person's and one an agent queued, both waiting for the slot.
async fn person_and_agent(harness: &Harness, engine: &JobEngine) -> (Uuid, Uuid) {
    let running = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_reached(running.id, &JobState::Queued)
        .await;
    let persons = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap()
        .id;
    let options = EnqueueOptions {
        agent: Some(AgentOrigin::new(None, "r-1", cents(500))),
        ..EnqueueOptions::default()
    };
    let agents = engine.enqueue(make_spec(1), options).await.unwrap().id;
    waiting_for_slot(harness, 2).await;
    (persons, agents)
}

#[tokio::test]
async fn pause_all_submits_nothing_new_until_resume_all_while_submitted_jobs_finish() {
    let harness = holding(1);
    let engine = harness.engine().await;
    let running = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_reached(running.id, &JobState::Queued)
        .await;
    let mut waiting = Vec::new();
    for _ in 0..5 {
        waiting.push(
            engine
                .enqueue(make_spec(1), EnqueueOptions::default())
                .await
                .unwrap()
                .id,
        );
    }
    waiting_for_slot(&harness, 5).await;

    engine.pause_all(false).await;
    for id in &waiting {
        assert_eq!(state(&harness, *id), by_user());
    }

    harness.provider.with(|state| state.held_status = None);
    harness.repository.wait_reached(running.id, &JobState::Done).await;
    // `Done` is saved after the slot is released, so no settle is needed.
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 1);
    assert_eq!(engine.limiter().in_use(), 0);
    for id in &waiting {
        assert_eq!(state(&harness, *id), by_user());
    }

    engine.resume_all(false).await;
    for id in &waiting {
        harness.repository.wait_reached(*id, &JobState::Done).await;
    }
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 6);
    assert_eq!(harness.provider.with(|state| state.max_in_flight), 1);
}

/// An agent's pause_queue and resume_queue touch only the jobs agents queued, and never lift the person's own Pause
/// all, whether it came before the agent's pause or after it.
#[tokio::test]
async fn an_agents_pause_touches_only_agents_jobs_and_never_lifts_the_persons() {
    let harness = holding(1);
    let engine = harness.engine().await;
    let (persons, agents) = person_and_agent(&harness, &engine).await;

    engine.pause_all(true).await;
    assert_eq!(state(&harness, persons), JobState::WaitingForSlot);
    assert_eq!(state(&harness, agents), by_user());
    engine.resume_all(true).await;
    waiting_for_slot(&harness, 2).await;

    engine.pause_all(false).await;
    engine.resume_all(true).await;
    quiesce().await;
    assert_eq!(state(&harness, persons), by_user());
    assert_eq!(state(&harness, agents), by_user());

    engine.resume_all(false).await;
    waiting_for_slot(&harness, 2).await;
    engine.pause_all(true).await;
    engine.pause_all(false).await;
    engine.resume_all(true).await;
    quiesce().await;
    assert_eq!(state(&harness, agents), by_user());
    assert_eq!(harness.log.count("submit"), 1);
}

/// The person's Pause agents pauses only the agents' jobs, and no agent's resume_queue lifts it; Resume agents does,
/// and leaves the person's own Pause all alone.
#[tokio::test]
async fn the_persons_pause_of_agents_holds_against_an_agents_resume() {
    let harness = holding(1);
    let engine = harness.engine().await;
    let (persons, agents) = person_and_agent(&harness, &engine).await;

    engine.pause_all(true).await;
    engine.pause_agent_jobs().await;
    engine.resume_all(true).await;
    quiesce().await;
    assert_eq!(state(&harness, persons), JobState::WaitingForSlot);
    assert_eq!(state(&harness, agents), by_user());

    engine.resume_agent_jobs().await;
    waiting_for_slot(&harness, 2).await;
    assert_eq!(harness.log.count("submit"), 1);
}

#[tokio::test]
async fn pause_by_user_survives_a_restart_and_a_connection_change() {
    let harness = Harness::new();
    harness.connectivity.set(false);
    let first = harness.engine().await;
    let job = first
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_reached(job.id, &JobState::Uploading)
        .await;

    first.pause_all(false).await;
    first.set_provider(Some(Arc::new(harness.provider.clone()))).await;
    assert_eq!(state(&harness, job.id), by_user());
    first.stop(Duration::ZERO).await;

    harness.connectivity.set(true);
    let second = harness.engine().await;
    second
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    quiesce().await;
    assert_eq!(state(&harness, job.id), by_user());
    assert!(harness.provider.with(|state| state.uploads.is_empty()));

    second.resume_all(false).await;
    harness.repository.wait_reached(job.id, &JobState::Done).await;
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key]
    );
}

#[tokio::test]
async fn pause_all_takes_over_a_connection_pause_and_resume_all_keeps_its_cause() {
    let harness = Harness::new();
    harness.connectivity.set(false);
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_reached(job.id, &JobState::Uploading)
        .await;
    let other = FakeProvider::new(CallLog::default());
    other.with(|state| state.kind = Some(ProviderKind::Account));
    engine.set_provider(Some(Arc::new(other.clone()))).await;
    let attention = paused(PauseReason::ConnectionNeedsAttention);
    assert_eq!(state(&harness, job.id), attention);

    engine.pause_all(false).await;
    harness.connectivity.set(true);
    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    quiesce().await;
    assert_eq!(state(&harness, job.id), by_user());

    engine.set_provider(Some(Arc::new(other))).await;
    engine.resume_all(false).await;
    harness
        .repository
        .wait_until(|repository| {
            repository
                .history(job.id)
                .iter()
                .filter(|saved| **saved == attention)
                .count()
                == 2
        })
        .await;
    engine
        .set_provider(Some(Arc::new(harness.provider.clone())))
        .await;
    harness.repository.wait_reached(job.id, &JobState::Done).await;
}

#[tokio::test]
async fn funds_pauses_stay_through_pause_all_and_resume_all() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(
            ProviderError::InsufficientFunds,
        ))]
        .into();
    });
    let engine = harness.engine().await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let funds = paused(PauseReason::InsufficientFunds);
    harness.repository.wait_reached(job.id, &funds).await;

    engine.pause_all(false).await;
    engine.resume_all(false).await;
    quiesce().await;
    assert_eq!(state(&harness, job.id), funds);
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key]
    );
}

#[tokio::test]
async fn a_submit_already_sent_is_not_paused() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_steps = [SubmitStep::Hang].into());
    let first = harness.engine().await;
    let job = first
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    eventually(|| harness.provider.with(|state| state.submit_keys.len()) == 1).await;

    first.pause_all(false).await;
    assert_eq!(state(&harness, job.id), JobState::WaitingForSlot);
    first.stop(Duration::ZERO).await;

    let _second = harness.engine().await;
    harness.repository.wait_reached(job.id, &JobState::Done).await;
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key, job.idempotency_key]
    );
    assert_eq!(harness.provider.with(|state| state.accepted.len()), 1);
}
