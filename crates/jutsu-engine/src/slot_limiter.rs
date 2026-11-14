//! The account's generation slots.

use std::cmp::Ordering;
use std::sync::{Mutex, MutexGuard, PoisonError};

use time::OffsetDateTime;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Async semaphore for the account's generation slots. Waiters are served in `SlotOrder`, and the limit can change
/// at any time, as when the user changes the setting.
#[derive(Debug)]
pub struct SlotLimiter {
    state: Mutex<LimiterState>,
}

#[derive(Debug)]
struct LimiterState {
    limit: usize,
    in_use: usize,
    /// Sorted by `order`, then by arrival.
    waiters: Vec<Waiter>,
}

#[derive(Debug)]
struct Waiter {
    id: Uuid,
    /// Tells apart two waits of the same ID.
    ticket: Uuid,
    order: SlotOrder,
    grant: oneshot::Sender<()>,
}

/// Where a job waits for a slot: promoted jobs first, the latest promotion ahead, then the rest oldest first. Built
/// from saved dates only, so jobs resumed after a restart line up as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotOrder {
    /// When the job was queued.
    pub queued_at: OffsetDateTime,
    /// When the person moved it ahead (Run next).
    pub promoted_at: Option<OffsetDateTime>,
}

impl SlotOrder {
    /// A job queued at `queued_at`, promoted at `promoted_at`.
    pub fn new(queued_at: OffsetDateTime, promoted_at: Option<OffsetDateTime>) -> Self {
        Self {
            queued_at,
            promoted_at,
        }
    }

    /// Whether `self` is served before `other`.
    fn precedes(&self, other: &Self) -> bool {
        match (self.promoted_at, other.promoted_at) {
            (Some(left), Some(right)) => left > right,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => self.queued_at < other.queued_at,
        }
    }
}

impl PartialOrd for SlotOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(if self.precedes(other) {
            Ordering::Less
        } else if other.precedes(self) {
            Ordering::Greater
        } else {
            Ordering::Equal
        })
    }
}

impl SlotLimiter {
    /// A limiter of `limit` slots, at least 1.
    pub fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(LimiterState {
                limit: limit.max(1),
                in_use: 0,
                waiters: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, LimiterState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The limit.
    pub fn limit(&self) -> usize {
        self.lock().limit
    }

    /// Slots taken.
    pub fn in_use(&self) -> usize {
        self.lock().in_use
    }

    /// How many wait.
    pub fn waiter_count(&self) -> usize {
        self.lock().waiters.len()
    }

    /// The waiting IDs, next to be served first.
    pub fn waiting_ids(&self) -> Vec<Uuid> {
        self.lock().waiters.iter().map(|waiter| waiter.id).collect()
    }

    /// Waits for a free slot for `id`. Dropping the future while it waits leaves the queue; a slot granted to a
    /// dropped wait is given back.
    pub async fn acquire(&self, id: Uuid, order: SlotOrder) {
        let (grant, granted) = oneshot::channel();
        let ticket = Uuid::new_v4();
        {
            let mut state = self.lock();
            if state.waiters.is_empty() && state.in_use < state.limit {
                state.in_use += 1;
                return;
            }
            Self::insert(
                &mut state,
                Waiter {
                    id,
                    ticket,
                    order,
                    grant,
                },
            );
        }
        let mut wait = Wait {
            limiter: self,
            ticket,
            granted: Some(granted),
        };
        if let Some(granted) = wait.granted.as_mut() {
            // The sender lives until it grants or the wait leaves the queue (on drop), so this returns once granted.
            let _ = granted.await;
        }
        wait.granted = None;
    }

    /// Moves a waiting `id` ahead of the others as promoted at `date`. Does nothing when it isn't waiting.
    pub fn promote(&self, id: Uuid, date: OffsetDateTime) {
        let mut state = self.lock();
        let Some(index) = state.waiters.iter().position(|waiter| waiter.id == id) else {
            return;
        };
        let mut waiter = state.waiters.remove(index);
        waiter.order.promoted_at = Some(date);
        Self::insert(&mut state, waiter);
    }

    /// Takes a slot even when none is free: a job the API already accepted holds one regardless.
    pub fn occupy(&self) {
        self.lock().in_use += 1;
    }

    /// Gives a slot back.
    pub fn release(&self) {
        let mut state = self.lock();
        state.in_use = state.in_use.saturating_sub(1);
        Self::grant(&mut state);
    }

    /// Changes the limit, at least 1.
    pub fn set_limit(&self, limit: usize) {
        let mut state = self.lock();
        state.limit = limit.max(1);
        Self::grant(&mut state);
    }

    fn insert(state: &mut LimiterState, waiter: Waiter) {
        let index = state
            .waiters
            .iter()
            .position(|other| waiter.order.precedes(&other.order))
            .unwrap_or(state.waiters.len());
        state.waiters.insert(index, waiter);
    }

    fn grant(state: &mut LimiterState) {
        while state.in_use < state.limit && !state.waiters.is_empty() {
            let waiter = state.waiters.remove(0);
            state.in_use += 1;
            if waiter.grant.send(()).is_err() {
                // The wait is gone; its slot is free again.
                state.in_use -= 1;
            }
        }
    }
}

/// A wait in the queue; dropped before it is granted, it leaves the queue.
struct Wait<'a> {
    limiter: &'a SlotLimiter,
    ticket: Uuid,
    granted: Option<oneshot::Receiver<()>>,
}

impl Drop for Wait<'_> {
    fn drop(&mut self) {
        let Some(mut granted) = self.granted.take() else {
            return;
        };
        let mut state = self.limiter.lock();
        if let Some(index) = state
            .waiters
            .iter()
            .position(|waiter| waiter.ticket == self.ticket)
        {
            state.waiters.remove(index);
            return;
        }
        // Granted after the wait was given up: the slot goes to the next waiter.
        if granted.try_recv().is_ok() {
            state.in_use = state.in_use.saturating_sub(1);
            SlotLimiter::grant(&mut state);
        }
    }
}
