//! The server's quote right before submit, the submit's own answer and the charge after the run: a port of
//! `ServerPriceTests`.

use std::sync::{Arc, Mutex};

use jutsu_core::{
    BudgetError, Cost, FailureReason, JobState, PauseReason, PortError, ProviderError, Rejection,
    RemoteStatus,
};
use url::Url;

use super::{Harness, credits, failed, make_spec, paused, read_spend, spend, usd};
use crate::EnqueueOptions;

fn approved() -> EnqueueOptions {
    EnqueueOptions {
        over_cap_approved: true,
        ..EnqueueOptions::default()
    }
}

fn server_error() -> PortError {
    PortError::Provider(ProviderError::Server { status: 503 })
}

#[tokio::test]
async fn spending_that_becomes_unreadable_after_enqueue_prevents_submission() {
    let harness = Harness::new();
    let reads = Arc::new(Mutex::new(0));
    let mut deps = harness.deps(None);
    deps.spend_status = Some(read_spend(move || {
        let mut count = reads.lock().unwrap();
        *count += 1;
        if *count > 1 {
            return Err(BudgetError::SpendingUnavailable);
        }
        Ok(Some(spend("0", "10")))
    }));
    let engine = harness.engine_with(deps).await;
    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();

    harness
        .repository
        .wait_state(
            job.id,
            &failed(FailureReason::Message(
                BudgetError::SpendingUnavailable.to_string(),
            )),
        )
        .await;
    assert!(harness.provider.with(|state| state.submit_keys.is_empty()));
}

#[tokio::test]
async fn quote_is_kept_next_to_the_preliminary_price_and_counts_for_the_cap() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("0", "10"));
    harness.provider.with(|state| state.quote = Some(usd("1.25")));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.cost, usd("1.30"));
    assert_eq!(done.server_price, Some(usd("1.25")));
    assert_eq!(*done.price(), usd("1.25"));
    let log = harness.log.entries();
    let quoted = log.iter().position(|entry| entry == "serverEstimate").unwrap();
    assert!(quoted > log.iter().rposition(|entry| entry == "upload").unwrap());
    assert!(quoted < log.iter().position(|entry| entry == "submit").unwrap());
}

#[tokio::test]
async fn quote_over_the_cap_pauses_until_the_user_agrees() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("1", "3"));
    harness.provider.with(|state| state.quote = Some(usd("2.5")));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let held = harness
        .repository
        .wait_state(job.id, &paused(PauseReason::PriceAboveCap))
        .await;

    assert_eq!(held.server_price, Some(usd("2.5")));
    assert_eq!(held.approved_price, None);
    assert!(harness.provider.with(|state| state.submit_keys.is_empty()));

    engine.resume(job.id).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;
    assert_eq!(done.approved_price, Some(usd("2.5")));
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key]
    );
    assert_eq!(harness.log.count("serverEstimate"), 1);
}

#[tokio::test]
async fn a_quote_within_the_price_agreed_over_the_cap_is_not_asked_again() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("2", "3"));
    harness.provider.with(|state| state.quote = Some(usd("1.25")));
    let engine = harness.engine().await;

    let job = engine.enqueue(make_spec(1), approved()).await.unwrap();

    assert_eq!(job.approved_price, Some(usd("1.30")));
    harness.repository.wait_state(job.id, &JobState::Done).await;
    assert!(
        !harness
            .repository
            .history(job.id)
            .contains(&paused(PauseReason::PriceAboveCap))
    );
}

/// Agreeing to go over the cap by the estimate isn't agreeing to any price the server quotes later.
#[tokio::test]
async fn a_quote_above_the_price_agreed_over_the_cap_is_asked_again() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("2", "3"));
    harness.provider.with(|state| state.quote = Some(usd("2.5")));
    let engine = harness.engine().await;

    let job = engine.enqueue(make_spec(1), approved()).await.unwrap();

    harness
        .repository
        .wait_state(job.id, &paused(PauseReason::PriceAboveCap))
        .await;
    assert!(harness.provider.with(|state| state.submit_keys.is_empty()));
}

#[tokio::test]
async fn failed_quote_falls_back_to_the_preliminary_price() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("0", "1"));
    harness
        .provider
        .with(|state| state.quote_error = Some(server_error()));
    let engine = harness.engine().await;

    let job = engine.enqueue(make_spec(1), approved()).await.unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.server_price, None);
    assert_eq!(*done.price(), usd("1.30"));
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [job.idempotency_key]
    );
}

#[tokio::test]
async fn submit_answered_completed_downloads_without_polling() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.submit_status = Some(RemoteStatus::Completed {
            video_url: Url::parse("https://cdn.example/now.mp4").unwrap(),
        });
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(harness.log.count("status"), 0);
    assert!(done.charged_at.is_some());
    assert_eq!(done.remote.unwrap().initial_status, None);
}

#[tokio::test]
async fn submit_answered_rejected_ends_the_job_at_once() {
    let harness = Harness::new();
    harness
        .provider
        .with(|state| state.submit_status = Some(RemoteStatus::Rejected(Rejection::ExplicitContent)));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();

    harness.repository.wait_state(job.id, &JobState::Rejected).await;
    assert_eq!(harness.log.count("status"), 0);
    assert_eq!(engine.limiter().in_use(), 0);
}

/// A Topaz upscale is queued without a price; once it finishes, what the provider says it charged becomes its price,
/// and the cap counts it from then on.
#[tokio::test]
async fn an_unknown_price_is_filled_in_with_the_charge_once_the_job_finishes() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.preliminary = Some(Cost::UNKNOWN_CREDITS);
        state.charge = Some(credits(40));
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.cost, Cost::UNKNOWN_CREDITS);
    assert_eq!(done.server_price, Some(credits(40)));
    assert!(!done.price_is_unknown());
    assert_eq!(harness.log.count("charge"), 1);
}

#[tokio::test]
async fn a_charge_that_cant_be_read_leaves_the_price_unknown() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.preliminary = Some(Cost::UNKNOWN_CREDITS);
        state.charge_error = Some(server_error());
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(done.server_price, None);
    assert!(done.price_is_unknown());
}

#[tokio::test]
async fn a_known_price_is_never_looked_up_after_the_run() {
    let harness = Harness::new();
    harness.provider.with(|state| state.charge = Some(usd("9")));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(make_spec(1), EnqueueOptions::default())
        .await
        .unwrap();
    let done = harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(*done.price(), usd("1.30"));
    assert_eq!(harness.log.count("charge"), 0);
}
