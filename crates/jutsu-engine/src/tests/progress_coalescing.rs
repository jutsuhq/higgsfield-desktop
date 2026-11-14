//! Port of `ProgressCoalescingTests`. Swift's events carry the whole job; here progress arrives as
//! `EngineEvent::Progress` and every other change as `EngineEvent::JobChanged`, read the same way.

#![allow(clippy::float_cmp, reason = "the progress compared is the value reported")]

use jutsu_core::{Job, JobEvent, JobRepository, JobState, PauseReason, ProviderKind, SubmitAttempt};
use time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{Harness, make_spec, state_name, usd};
use crate::clock::EngineClock;
use crate::engine::EngineEvent;
use crate::{EnqueueOptions, JobEngine};

/// An event as Swift's test reads it: the job, its state and its progress, and the job when the event carries it.
fn read(event: EngineEvent) -> (Uuid, JobState, f64, Option<Job>) {
    match event {
        EngineEvent::JobChanged(job) => (job.id, job.state.clone(), job.progress, Some(*job)),
        EngineEvent::Progress {
            job_id,
            state,
            fraction,
        } => (job_id, state, fraction, None),
    }
}

async fn next(events: &mut broadcast::Receiver<EngineEvent>) -> (Uuid, JobState, f64, Option<Job>) {
    read(events.recv().await.unwrap())
}

/// The job's events up to the first one past preparing, as "<state> <progress>".
async fn preparing_events(id: Uuid, events: &mut broadcast::Receiver<EngineEvent>) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let (job, state, progress, _) = next(events).await;
        if job != id {
            continue;
        }
        seen.push(format!("{} {progress:?}", state_name(&state)));
        if state != JobState::Preparing {
            return seen;
        }
    }
}

async fn preparing_events_of(steps: Vec<f64>) -> Vec<String> {
    let mut harness = Harness::new();
    harness.media.progress_steps = steps;
    let engine = harness.engine().await;
    let mut events = engine.subscribe();

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();

    preparing_events(job.id, &mut events).await
}

#[tokio::test]
async fn a_flood_of_small_steps_yields_only_a_few_events() {
    let flood = (1..=1_000).map(|step| 0.5 + f64::from(step) * 0.000_005);
    let steps = std::iter::once(0.5).chain(flood).chain([1.0]).collect();

    assert_eq!(
        preparing_events_of(steps).await,
        ["preparing 0.0", "preparing 0.5", "preparing 1.0", "uploading 0.0"]
    );
}

#[tokio::test]
async fn start_and_end_are_always_delivered() {
    assert_eq!(
        preparing_events_of(vec![0.3, 0.0, 0.000_1, 0.999_9, 1.0, 1.0]).await,
        [
            "preparing 0.0",
            "preparing 0.3",
            "preparing 0.0",
            "preparing 0.9999",
            "preparing 1.0",
            "preparing 1.0",
            "uploading 0.0",
        ]
    );
}

fn paused() -> JobState {
    JobState::Paused(PauseReason::InsufficientFunds)
}

/// An engine holding paused jobs, which no runner touches, so the test alone reports their progress.
async fn paused_engine(harness: &Harness, count: usize) -> (JobEngine, Vec<Uuid>) {
    let mut ids = Vec::new();
    for _ in 0..count {
        let mut job = Job::new(
            Uuid::new_v4(),
            make_spec(1),
            harness
                .provider
                .with(|state| state.kind.unwrap_or(ProviderKind::ApiKey)),
            usd("1.30"),
            Uuid::new_v4(),
            harness.clock.now(),
        );
        job.owner = harness.provider.with(|state| state.owner.clone());
        job.state = paused();
        harness.repository.save(&job).await.unwrap();
        ids.push(job.id);
    }
    (harness.engine().await, ids)
}

#[tokio::test]
async fn small_steps_are_published_once_the_interval_has_passed() {
    let harness = Harness::new();
    let (engine, ids) = paused_engine(&harness, 1).await;
    let id = ids[0];
    let mut events = engine.subscribe();

    engine.report(0.5, id, &paused());
    engine.report(0.501, id, &paused());
    harness.clock.advance(Duration::milliseconds(300));
    engine.report(0.502, id, &paused());

    assert_eq!(next(&mut events).await.2, 0.5);
    assert_eq!(next(&mut events).await.2, 0.502);
}

#[tokio::test]
async fn a_change_after_coalesced_progress_is_delivered_at_once_with_the_latest_progress() {
    let harness = Harness::new();
    let (engine, ids) = paused_engine(&harness, 1).await;
    let id = ids[0];
    let mut events = engine.subscribe();

    engine.report(0.5, id, &paused());
    engine.report(0.504, id, &paused());
    engine
        .save(id, |job| {
            job.submit_attempt = Some(SubmitAttempt { is_idempotent: true });
        })
        .await
        .unwrap();
    engine.report(0.505, id, &paused());
    engine.apply(JobEvent::Resume, id, |_| {}).await.unwrap();

    let first = next(&mut events).await;
    let saved = next(&mut events).await;
    let resumed = next(&mut events).await;
    assert_eq!(first.2, 0.5);
    assert_eq!(saved.2, 0.504);
    assert!(saved.3.unwrap().submit_attempt.is_some());
    assert_eq!(resumed.1, JobState::Preparing);
    assert_eq!(resumed.2, 0.0);
}

#[tokio::test]
async fn one_jobs_progress_never_suppresses_anothers_events() {
    let harness = Harness::new();
    let (engine, ids) = paused_engine(&harness, 2).await;
    let (a, b) = (ids[0], ids[1]);
    let mut events = engine.subscribe();

    engine.report(0.5, a, &paused());
    engine.report(0.5, b, &paused());
    engine.report(0.502, a, &paused());
    engine.report(0.502, b, &paused());
    engine.report(0.6, b, &paused());
    engine.save(a, |_| {}).await.unwrap();

    let mut seen = Vec::new();
    for _ in 0..4 {
        let (job, _, progress, _) = next(&mut events).await;
        seen.push(format!("{} {progress:?}", if job == a { "a" } else { "b" }));
    }
    assert_eq!(seen, ["a 0.5", "b 0.5", "b 0.6", "a 0.502"]);
}
