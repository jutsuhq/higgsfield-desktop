//! The engine's time and randomness, both replaceable so tests run waits, deadlines and jitter without waiting.

use std::sync::Arc;

use async_trait::async_trait;
use time::{Duration, OffsetDateTime};

/// Time source of the engine; tests swap in a clock that doesn't really wait.
#[async_trait]
pub trait EngineClock: Send + Sync {
    /// Now.
    fn now(&self) -> OffsetDateTime;
    /// Waits `duration`. The engine cuts the wait off when the job's task is cancelled.
    async fn sleep(&self, duration: Duration);
}

/// The system's clock and tokio's timer.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

#[async_trait]
impl EngineClock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    async fn sleep(&self, duration: Duration) {
        let wait =
            std::time::Duration::try_from(duration.max(Duration::ZERO)).unwrap_or(std::time::Duration::MAX);
        tokio::time::sleep(wait).await;
    }
}

/// A random number in 0…1: the jitter of the engine's waits.
pub type Random = Arc<dyn Fn() -> f64 + Send + Sync>;

/// A random number in 0…1 from the system's randomness (through a v4 UUID).
pub fn system_random() -> Random {
    Arc::new(|| {
        let bits = uuid::Uuid::new_v4().as_u64_pair().0 >> 11;
        #[allow(clippy::cast_precision_loss, reason = "53 bits fit a double exactly")]
        let unit = bits as f64 / (1_u64 << 53) as f64;
        unit
    })
}
