//! A double click on Generate sends two enqueues before the first is saved: a port of `EnqueueTests`.

use std::sync::Arc;

use jutsu_core::{
    AspectRatio, Budget, BudgetError, CostUnit, JobRepository, JobSpec, JobState, LocalImage, Mode,
    PixelSize, Resolution, SpendCap, SpendStatus, VideoModel,
};
use rust_decimal::Decimal;
use time::UtcOffset;
use url::Url;
use uuid::Uuid;

use super::{Gate, Harness, make_spec, quiesce, read_spend, spend};
use crate::clock::EngineClock;
use crate::{EngineError, EnqueueOptions};

fn approved(over_cap_approved: bool) -> EnqueueOptions {
    EnqueueOptions {
        over_cap_approved,
        ..EnqueueOptions::default()
    }
}

#[tokio::test]
async fn unreadable_spending_sends_nothing_even_with_an_earlier_cap_approval() {
    for approval in [false, true] {
        let harness = Harness::new();
        let mut deps = harness.deps(None);
        deps.spend_status = Some(read_spend(|| Err(BudgetError::SpendingUnavailable)));
        let engine = harness.engine_with(deps).await;

        assert_eq!(
            engine.enqueue(make_spec(1), approved(approval)).await,
            Err(EngineError::Budget(BudgetError::SpendingUnavailable)),
            "approved: {approval}"
        );
        assert_eq!(
            harness.repository.active_jobs().await.unwrap(),
            [] as [jutsu_core::Job; 0]
        );
        assert!(harness.provider.with(|state| state.submit_keys.is_empty()));
    }
}

#[tokio::test]
async fn the_same_request_sent_while_the_first_is_queueing_makes_one_job() {
    let harness = Harness::new();
    let engine = harness.engine().await;
    let spec = make_spec(1);
    let (reached, release) = harness
        .repository
        .hold_save(|job| job.state == JobState::Preparing);

    let first = tokio::spawn({
        let engine = engine.clone();
        let spec = spec.clone();
        async move { engine.enqueue(spec, EnqueueOptions::default()).await }
    });
    reached.wait().await;
    let second = tokio::spawn({
        let engine = engine.clone();
        let spec = spec.clone();
        let options = EnqueueOptions {
            batch_id: Some(Uuid::new_v4()),
            ..EnqueueOptions::default()
        };
        async move { engine.enqueue(spec, options).await }
    });
    // The second has to reach the engine while the first is held; nothing observable marks that.
    quiesce().await;
    release.open();
    let a = first.await.unwrap().unwrap();
    let b = second.await.unwrap().unwrap();

    assert_eq!(a.id, b.id);
    assert_eq!(harness.log.count("estimate"), 1);
    harness.repository.wait_reached(a.id, &JobState::Done).await;
    assert_eq!(
        harness.provider.with(|state| state.submit_keys.clone()),
        [a.idempotency_key]
    );

    let again = engine.enqueue(spec, EnqueueOptions::default()).await.unwrap();
    assert_ne!(again.id, a.id);
}

#[tokio::test]
async fn two_submits_that_read_the_spend_at_once_cant_both_pass_the_cap() {
    let harness = Harness::new();
    let repository = harness.repository.clone();
    let log = harness.log.clone();
    let clock = harness.clock.clone();
    let first_read = Gate::new();
    let release = Gate::new();
    let mut deps = harness.deps(None);
    deps.spend_status = Some(Arc::new({
        let first_read = first_read.clone();
        let release = release.clone();
        move |job| {
            let repository = repository.clone();
            let log = log.clone();
            let clock = clock.clone();
            let first_read = first_read.clone();
            let release = release.clone();
            Box::pin(async move {
                let others: Vec<_> = repository
                    .active_jobs()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|other| other.id != job.id)
                    .collect();
                let committed = Budget::committed(&others, clock.now(), CostUnit::Usd, UtcOffset::UTC);
                log.push("spendStatus");
                if log.count("spendStatus") == 1 {
                    first_read.open();
                    release.wait().await;
                }
                Ok(Some(SpendStatus {
                    committed_today: committed,
                    cap: Some(SpendCap::usd(Decimal::from(2))),
                }))
            })
        }
    }));
    let engine = harness.engine_with(deps).await;

    let first = tokio::spawn({
        let engine = engine.clone();
        async move { engine.enqueue(make_spec(1), EnqueueOptions::default()).await }
    });
    first_read.wait().await;
    let second = tokio::spawn({
        let engine = engine.clone();
        async move { engine.enqueue(make_spec(2), EnqueueOptions::default()).await }
    });
    // Without the engine taking turns, the second would read the same spend now and pass too.
    quiesce().await;
    release.open();

    let queued = first.await.unwrap().unwrap();
    let refused = second.await.unwrap();
    assert!(matches!(refused, Err(EngineError::OverCap(_))), "{refused:?}");
    let active: Vec<_> = harness
        .repository
        .active_jobs()
        .await
        .unwrap()
        .into_iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(active, [queued.id]);
}

#[tokio::test]
async fn over_the_cap_only_an_approved_enqueue_queues() {
    let mut harness = Harness::new();
    harness.spend_status = Some(spend("2", "3"));
    let engine = harness.engine().await;

    // The refusal says what the cap counted: the cap, the rest of today and the job's own price.
    let refused = engine.enqueue(make_spec(1), EnqueueOptions::default()).await;
    let Err(EngineError::OverCap(over)) = refused else {
        panic!("not over the cap: {refused:?}");
    };
    assert_eq!(over.cap, SpendCap::usd(Decimal::from(3)));
    assert_eq!(over.committed_today, Decimal::from(2));
    assert_eq!(over.price.unit(), CostUnit::Usd);
    assert!(over.price.amount() > Decimal::ONE);
    assert_eq!(
        harness.repository.active_jobs().await.unwrap(),
        [] as [jutsu_core::Job; 0]
    );

    let job = engine.enqueue(make_spec(1), approved(true)).await.unwrap();
    harness.repository.wait_reached(job.id, &JobState::Done).await;
}

/// The account's Seedance makes 16:9 from a 9:16 start frame unless it is told the frame (live, 2026-10-15), so a
/// clip from frames is queued with the start frame's shape; one whose frame can't be read keeps none.
#[tokio::test]
async fn a_clip_from_frames_takes_the_start_frames_shape() {
    let cases = [
        (Some(PixelSize::new(480, 854)), Some(AspectRatio::Portrait)),
        (Some(PixelSize::new(1000, 1000)), Some(AspectRatio::Square)),
        (Some(PixelSize::new(2100, 900)), Some(AspectRatio::Cinema)),
        (None, None),
    ];
    for (size, expected) in cases {
        let mut harness = Harness::new();
        harness.media.image_size = size;
        let engine = harness.engine().await;
        let mut spec = JobSpec::new(
            Mode::MotionTransfer,
            VideoModel::SEEDANCE25,
            None,
            Resolution::P480,
        );
        spec.references = vec![LocalImage::new(
            Url::from_file_path(std::env::temp_dir().join("first.png")).unwrap(),
        )];
        spec.length = Some(4.0);
        spec.references_are_frames = true;

        let queued = engine
            .enqueue(spec.clone(), EnqueueOptions::default())
            .await
            .unwrap();
        assert_eq!(queued.spec.aspect_ratio, expected, "{size:?}");

        // References aren't frames: the frame asked for stays.
        let mut references = spec;
        references.references_are_frames = false;
        references.aspect_ratio = Some(AspectRatio::Landscape);
        let queued = engine
            .enqueue(references, EnqueueOptions::default())
            .await
            .unwrap();
        assert_eq!(queued.spec.aspect_ratio, Some(AspectRatio::Landscape));
    }
}
