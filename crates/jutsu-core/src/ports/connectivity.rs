//! Network reachability.

use futures::stream::BoxStream;

/// Network reachability. The engine pauses network work while offline.
pub trait Connectivity: Send + Sync {
    /// The current state first, then every change.
    fn updates(&self) -> BoxStream<'static, bool>;
}
