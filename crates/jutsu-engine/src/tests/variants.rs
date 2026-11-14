//! One submit of several variants (`JobSpec::variants`) becomes one job per variant, grouped as a batch: a port of
//! `VariantsTests`.

use std::collections::HashSet;

use jutsu_core::{FailureReason, Job, JobSpec, JobState, PortError, ProviderError};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{Harness, SubmitStep, credits, failed, make_spec, quiesce, usd};
use crate::EnqueueOptions;

fn variant_spec(variants: i64) -> JobSpec {
    let mut spec = make_spec(1);
    spec.variants = variants;
    spec
}

fn batch(harness: &Harness, batch_id: Option<Uuid>) -> Vec<Job> {
    harness
        .repository
        .all()
        .into_iter()
        .filter(|job| job.batch_id == batch_id)
        .collect()
}

async fn wait_done(harness: &Harness, batch_id: Option<Uuid>, count: usize) {
    harness
        .repository
        .wait_until(|_| {
            batch(harness, batch_id)
                .iter()
                .filter(|job| job.state == JobState::Done)
                .count()
                == count
        })
        .await;
}

fn request_ids(jobs: &[Job]) -> HashSet<String> {
    jobs.iter()
        .filter_map(|job| job.remote.as_ref().map(|remote| remote.request_id.clone()))
        .collect()
}

fn total_price(jobs: &[Job]) -> Decimal {
    jobs.iter().map(|job| job.price().amount()).sum()
}

#[tokio::test]
async fn one_submit_becomes_a_job_per_variant_in_one_batch() {
    let harness = Harness::new();
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(3), EnqueueOptions::default())
        .await
        .unwrap();
    assert!(job.batch_id.is_some());
    assert_eq!(job.cost, usd("3.9"));
    wait_done(&harness, job.batch_id, 3).await;

    let jobs = batch(&harness, job.batch_id);
    assert_eq!(harness.log.count("submit"), 1);
    assert_eq!(jobs.len(), 3);
    assert!(jobs.iter().all(|job| job.variant_group_id.is_some()));
    let groups: HashSet<_> = jobs.iter().filter_map(|job| job.variant_group_id).collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        request_ids(&jobs),
        HashSet::from(["r1".into(), "r1-v1".into(), "r1-v2".into()])
    );
    assert!(jobs.iter().all(|job| job.spec.variants == 1));
    assert!(jobs.iter().all(|job| job.output_file.is_some()));
    assert!(jobs.iter().all(|job| job.missing_variants.is_none()));
    assert_eq!(total_price(&jobs), "3.9".parse::<Decimal>().unwrap());
}

/// The quote is for the whole request: the jobs share it, so together they count it once.
#[tokio::test]
async fn the_requests_quote_is_shared_not_repeated() {
    let harness = Harness::new();
    harness.provider.with(|state| state.quote = Some(credits(10)));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(3), EnqueueOptions::default())
        .await
        .unwrap();
    wait_done(&harness, job.batch_id, 3).await;

    let jobs = batch(&harness, job.batch_id);
    let sender = jobs.iter().find(|other| other.id == job.id).unwrap();
    assert_eq!(sender.server_price, Some(credits(4)));
    let shares: Decimal = jobs
        .iter()
        .filter_map(|job| job.server_price.as_ref().map(jutsu_core::Cost::amount))
        .sum();
    assert_eq!(shares, Decimal::from(10));
    assert_eq!(harness.log.count("serverEstimate"), 1);
}

#[tokio::test]
async fn fewer_jobs_than_asked_are_tracked_and_noted() {
    let harness = Harness::new();
    harness.provider.with(|state| state.variants_started = Some(2));
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(4), EnqueueOptions::default())
        .await
        .unwrap();
    wait_done(&harness, job.batch_id, 2).await;
    quiesce().await;

    let jobs = batch(&harness, job.batch_id);
    let sender = jobs.iter().find(|other| other.id == job.id).unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(sender.missing_variants, Some(2));
    assert_eq!(request_ids(&jobs), HashSet::from(["r1".into(), "r1-v1".into()]));
    // Two of four shares: what the two started variants cost, not the whole request.
    assert_eq!(total_price(&jobs), "2.6".parse::<Decimal>().unwrap());
}

/// Every job the answer names is saved and polling before the job that sent it takes its own answer, so a crash in
/// between can't lose a paid variant.
#[tokio::test]
async fn variants_are_saved_before_the_senders_answer() {
    let harness = Harness::new();
    let (reached, release) = harness.repository.hold_save(|job| {
        job.state == JobState::Queued
            && job.spec.variants == 1
            && job
                .remote
                .as_ref()
                .is_some_and(|remote| remote.request_id == "r1")
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(3), EnqueueOptions::default())
        .await
        .unwrap();
    reached.wait().await;

    let variants: Vec<Job> = batch(&harness, job.batch_id)
        .into_iter()
        .filter(|other| other.id != job.id)
        .collect();
    assert_eq!(variants.len(), 2);
    assert!(
        variants
            .iter()
            .all(|variant| variant.remote.is_some() && variant.submitted_at.is_some())
    );
    release.open();
    wait_done(&harness, job.batch_id, 3).await;
}

/// The account caps an unlimited request to one generation, so the job asks for one.
#[tokio::test]
async fn the_unlimited_allowance_makes_one() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.pays_unlimited = true;
        state.unlimited_answer = (true, None);
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(3), EnqueueOptions::default())
        .await
        .unwrap();
    harness.repository.wait_state(job.id, &JobState::Done).await;

    assert_eq!(job.spec.variants, 1);
    assert_eq!(job.batch_id, None);
    assert_eq!(harness.repository.all().len(), 1);
}

/// A submit of variants is sent once, like any other: an unknown outcome fails it without sending it again.
#[tokio::test]
async fn an_unknown_outcome_is_not_sent_again() {
    let harness = Harness::new();
    harness.provider.with(|state| {
        state.not_idempotent = true;
        state.submit_steps = [SubmitStep::Fail(PortError::Provider(
            ProviderError::OutcomeUnknown,
        ))]
        .into();
    });
    let engine = harness.engine().await;

    let job = engine
        .enqueue(variant_spec(4), EnqueueOptions::default())
        .await
        .unwrap();
    harness
        .repository
        .wait_state(job.id, &failed(FailureReason::OutcomeUnknown))
        .await;
    quiesce().await;

    assert_eq!(harness.log.count("submit"), 1);
    assert_eq!(harness.repository.all().len(), 1);
}
