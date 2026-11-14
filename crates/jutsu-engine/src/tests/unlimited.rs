//! Jobs the plan's unlimited allowance pays for: a port of `UnlimitedTests`.

use jutsu_core::JobState;

use super::{Harness, credits, make_spec, spend, usd};
use crate::{EngineError, EnqueueOptions};

#[tokio::test]
async fn a_job_the_allowance_pays_is_neither_capped_nor_quoted() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("9", "1"));
    harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (true, None);
        state.quote = Some(usd("5"));
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert!(job.spec.uses_unlimited);
    assert!(done.spec.uses_unlimited);
    assert_eq!(done.server_price, None);
    assert_eq!(harness.log.count("serverEstimate"), 0);
}

#[tokio::test]
async fn the_provider_decides_not_the_copied_spec() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("9", "1"));
    let engine = harness.engine().await;
    let mut spec = make_spec(1);
    spec.uses_unlimited = true;

    let refused = engine.enqueue(spec.clone(), EnqueueOptions::default()).await;
    assert!(matches!(refused, Err(EngineError::OverCap(_))), "{refused:?}");

    let options = EnqueueOptions {
        over_cap_approved: true,
        ..EnqueueOptions::default()
    };
    let job = engine.enqueue(spec, options).await.unwrap();
    assert!(!job.spec.uses_unlimited);
}

#[tokio::test]
async fn credits_charged_after_all_are_recorded_and_counted() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (false, Some(credits(40)));
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert!(!done.spec.uses_unlimited);
    assert_eq!(done.server_price, Some(credits(40)));
    let remote = done.remote.unwrap();
    assert_eq!(remote.charged, None);
    assert!(!remote.paid_with_unlimited);
}

#[tokio::test]
async fn sent_without_the_allowance_keeps_the_estimate() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (false, None);
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert!(!done.spec.uses_unlimited);
    assert_eq!(*done.price(), usd("1.30"));
}
