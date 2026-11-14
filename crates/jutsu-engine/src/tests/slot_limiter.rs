//! Port of `SlotLimiterTests`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use time::{Duration, OffsetDateTime};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::eventually;
use crate::settings::Waits;
use crate::{EngineSettings, SlotLimiter, SlotOrder, system_random};

/// Foundation's `Date.distantPast`: 1 January of year 1.
fn distant_past() -> SlotOrder {
    SlotOrder::new(
        OffsetDateTime::from_unix_timestamp(-62_135_596_800).unwrap(),
        None,
    )
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

fn queued_at(seconds: i64) -> SlotOrder {
    SlotOrder::new(at(seconds), None)
}

/// The order waiters were served in.
type Served = Arc<Mutex<Vec<usize>>>;

fn served(log: &Served) -> Vec<usize> {
    log.lock().unwrap().clone()
}

fn wait_for_slot(
    limiter: &Arc<SlotLimiter>,
    id: Uuid,
    order: SlotOrder,
    served: &Served,
    value: usize,
) -> JoinHandle<()> {
    let (limiter, served) = (limiter.clone(), served.clone());
    tokio::spawn(async move {
        limiter.acquire(id, order).await;
        served.lock().unwrap().push(value);
    })
}

/// Holds one slot and queues one waiter per order.
async fn queue(
    orders: &[SlotOrder],
    limiter: &Arc<SlotLimiter>,
    served: &Served,
) -> (Vec<Uuid>, Vec<JoinHandle<()>>) {
    limiter.acquire(Uuid::new_v4(), distant_past()).await;
    let mut ids = Vec::new();
    let mut tasks = Vec::new();
    for (index, order) in orders.iter().enumerate() {
        let id = Uuid::new_v4();
        ids.push(id);
        tasks.push(wait_for_slot(limiter, id, *order, served, index));
        eventually(|| limiter.waiter_count() == index + 1).await;
    }
    (ids, tasks)
}

async fn finish(tasks: Vec<JoinHandle<()>>) {
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
}

#[tokio::test]
async fn waiters_are_served_in_arrival_order() {
    let limiter = Arc::new(SlotLimiter::new(1));
    let order = Served::default();
    limiter.acquire(Uuid::new_v4(), distant_past()).await;

    let first = wait_for_slot(&limiter, Uuid::new_v4(), distant_past(), &order, 1);
    eventually(|| limiter.waiter_count() == 1).await;
    let second = wait_for_slot(&limiter, Uuid::new_v4(), distant_past(), &order, 2);
    eventually(|| limiter.waiter_count() == 2).await;

    limiter.release();
    first.await.unwrap();
    assert_eq!(served(&order), [1]);
    limiter.release();
    second.await.unwrap();
    assert_eq!(served(&order), [1, 2]);
}

#[tokio::test]
async fn a_cancelled_waiter_leaves_the_queue_and_keeps_no_slot() {
    let limiter = Arc::new(SlotLimiter::new(1));
    limiter.acquire(Uuid::new_v4(), queued_at(0)).await;
    let waiting = {
        let limiter = limiter.clone();
        tokio::spawn(async move { limiter.acquire(Uuid::new_v4(), queued_at(1)).await })
    };
    eventually(|| limiter.waiter_count() == 1).await;
    waiting.abort();
    let _ = waiting.await;
    assert_eq!(limiter.waiter_count(), 0);
    limiter.release();
    assert_eq!(limiter.in_use(), 0);
}

#[tokio::test]
async fn raising_the_limit_lets_waiters_in_and_lowering_it_takes_effect_on_release() {
    let limiter = Arc::new(SlotLimiter::new(1));
    limiter.acquire(Uuid::new_v4(), queued_at(0)).await;
    let waiting = {
        let limiter = limiter.clone();
        tokio::spawn(async move { limiter.acquire(Uuid::new_v4(), queued_at(1)).await })
    };
    eventually(|| limiter.waiter_count() == 1).await;
    limiter.set_limit(2);
    waiting.await.unwrap();
    assert_eq!(limiter.in_use(), 2);

    limiter.set_limit(1);
    limiter.release();
    assert_eq!(limiter.in_use(), 1);
    limiter.occupy();
    assert_eq!(
        limiter.in_use(),
        2,
        "an accepted job holds its slot past the limit"
    );
}

#[tokio::test]
async fn waiters_are_served_oldest_first_whatever_the_arrival() {
    let limiter = Arc::new(SlotLimiter::new(1));
    let served = Served::default();
    let (ids, tasks) = queue(&[queued_at(30), queued_at(10), queued_at(20)], &limiter, &served).await;

    assert_eq!(limiter.waiting_ids(), [ids[1], ids[2], ids[0]]);
    finish(tasks).await;
}

#[tokio::test]
async fn promoted_waiter_takes_the_next_slot() {
    let limiter = Arc::new(SlotLimiter::new(2));
    limiter.acquire(Uuid::new_v4(), distant_past()).await;
    let served_log = Served::default();
    let orders: Vec<SlotOrder> = (1..=5).map(queued_at).collect();
    let (ids, tasks) = queue(&orders, &limiter, &served_log).await;

    limiter.promote(ids[4], at(100));
    assert_eq!(limiter.waiting_ids(), [ids[4], ids[0], ids[1], ids[2], ids[3]]);
    limiter.release();
    eventually(|| served(&served_log) == [4]).await;
    limiter.release();
    eventually(|| served(&served_log) == [4, 0]).await;
    finish(tasks).await;
}

#[tokio::test]
async fn latest_promotion_goes_first_and_promoting_an_idle_id_does_nothing() {
    let limiter = Arc::new(SlotLimiter::new(1));
    let served = Served::default();
    let orders: Vec<SlotOrder> = (1..=3).map(queued_at).collect();
    let (ids, mut tasks) = queue(&orders, &limiter, &served).await;

    limiter.promote(ids[1], at(100));
    limiter.promote(ids[2], at(200));
    limiter.promote(Uuid::new_v4(), at(300));
    assert_eq!(limiter.waiting_ids(), [ids[2], ids[1], ids[0]]);

    let late = Uuid::new_v4();
    tasks.push(wait_for_slot(
        &limiter,
        late,
        SlotOrder::new(at(0), Some(at(150))),
        &served,
        3,
    ));
    eventually(|| limiter.waiter_count() == 4).await;
    assert_eq!(limiter.waiting_ids(), [ids[2], late, ids[1], ids[0]]);
    finish(tasks).await;
}

#[tokio::test]
async fn canceling_a_promoted_waiter_serves_the_next_in_order() {
    let limiter = Arc::new(SlotLimiter::new(1));
    limiter.acquire(Uuid::new_v4(), distant_past()).await;
    let ignored = Served::default();
    let promoted = Uuid::new_v4();
    let waiter = wait_for_slot(&limiter, promoted, queued_at(2), &ignored, 0);
    eventually(|| limiter.waiter_count() == 1).await;
    let served_log = Served::default();
    let other = Uuid::new_v4();
    let other_task = wait_for_slot(&limiter, other, queued_at(1), &served_log, 1);
    eventually(|| limiter.waiter_count() == 2).await;
    limiter.promote(promoted, at(100));

    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(limiter.waiting_ids(), [other]);
    limiter.release();
    other_task.await.unwrap();
    assert_eq!(served(&served_log), [1]);
    assert_eq!(limiter.in_use(), 1);
}

#[tokio::test]
async fn raising_the_limit_admits_waiters_in_order() {
    let limiter = Arc::new(SlotLimiter::new(1));
    let served_log = Served::default();
    let orders: Vec<SlotOrder> = (1..=3).map(queued_at).collect();
    let (ids, tasks) = queue(&orders, &limiter, &served_log).await;
    limiter.promote(ids[2], at(100));

    limiter.set_limit(3);
    eventually(|| served(&served_log).len() == 2).await;
    assert_eq!(
        served(&served_log).into_iter().collect::<HashSet<_>>(),
        HashSet::from([2, 0])
    );
    assert_eq!(limiter.waiting_ids(), [ids[1]]);
    assert_eq!(limiter.in_use(), 3);
    finish(tasks).await;
}

fn settings(jitter: f64) -> EngineSettings {
    let mut settings = EngineSettings::new(PathBuf::from("output"));
    settings.jitter = jitter;
    settings
}

fn seconds(waits: &mut Waits, count: usize) -> Vec<f64> {
    (0..count).map(|_| waits.next().as_seconds_f64()).collect()
}

/// Higgsfield's polling guide: 2 s, growing by 1.5× up to 10 s.
#[test]
fn polling_follows_the_documented_schedule_and_resets() {
    let mut backoff = Waits::polling(&settings(0.0), system_random());
    assert_eq!(seconds(&mut backoff, 6), [2.0, 3.0, 4.5, 6.75, 10.0, 10.0]);
    backoff.reset();
    assert_eq!(backoff.next(), Duration::seconds(2));
}

#[test]
fn retries_double_to_the_cap() {
    let mut backoff = Waits::retrying(&settings(0.0), system_random());
    assert_eq!(seconds(&mut backoff, 6), [5.0, 10.0, 20.0, 40.0, 60.0, 60.0]);
}

#[test]
fn jitter_stays_within_bounds() {
    let mut backoff = Waits::polling(&settings(0.2), system_random());
    let first = backoff.next().as_seconds_f64();
    assert!((1.6..=2.4).contains(&first), "{first}");
}
