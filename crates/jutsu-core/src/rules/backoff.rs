//! Exponential backoff with jitter.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use time::Duration;

/// Exponential backoff with jitter: the waits between polls of a running job, and between retries of a failed
/// step. Shared by the job engine and the work outside the queue (Generate…, character sheets), which poll on
/// their own.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: f64,
    maximum: f64,
    factor: f64,
    jitter: f64,
    current: f64,
}

impl Backoff {
    /// Waits from `initial` growing by `factor` up to `maximum`; every wait is scaled by a random factor in
    /// `1 ± jitter`, 0–1.
    pub fn new(initial: Duration, maximum: Duration, factor: f64, jitter: f64) -> Self {
        let initial = initial.as_seconds_f64();
        Self {
            initial,
            maximum: initial.max(maximum.as_seconds_f64()),
            factor: factor.max(1.0),
            jitter: jitter.clamp(0.0, 1.0),
            current: initial,
        }
    }

    /// Higgsfield's polling guide: start at 2 s, grow by 1.5× up to 10 s.
    pub fn polling() -> Self {
        Self::polling_with(Duration::seconds(2), Duration::seconds(10), 0.0)
    }

    /// The polling guide's growth with other bounds and jitter.
    pub fn polling_with(initial: Duration, maximum: Duration, jitter: f64) -> Self {
        Self::new(initial, maximum, 1.5, jitter)
    }

    /// The next wait.
    pub fn next_wait(&mut self) -> Duration {
        let base = self.current;
        self.current = (self.current * self.factor).min(self.maximum);
        Duration::seconds_f64(base * (1.0 + self.jitter * random_signed_unit()))
    }

    /// Back to the first wait, as after a change of status.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

/// A random number in -1…1, from the standard library's randomly keyed hasher.
fn random_signed_unit() -> f64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(0x9E37_79B9_7F4A_7C15);
    let bits = hasher.finish() >> 11;
    (bits as f64 / (1_u64 << 53) as f64) * 2.0 - 1.0
}
