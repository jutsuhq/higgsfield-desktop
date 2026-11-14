//! Free quotes asked before upload, kept for as long as the connection lasts.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};

type Quote<V, E> = Shared<BoxFuture<'static, Result<V, E>>>;

/// One quote per key: a second estimate of the same job, even while the first is out, reads the same answer. A
/// failed quote isn't kept, so a later estimate asks again.
pub(crate) struct QuoteCache<K, V, E> {
    quotes: Arc<Mutex<HashMap<K, Quote<V, E>>>>,
}

impl<K, V, E> Default for QuoteCache<K, V, E> {
    fn default() -> Self {
        Self {
            quotes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<K, V, E> QuoteCache<K, V, E>
where
    K: Eq + Hash + Clone,
    V: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    /// The quote for `key`, asked with `ask` unless it was asked already. Returns the failure of `ask`.
    pub(crate) async fn value<F>(&self, key: K, ask: impl FnOnce() -> F) -> Result<V, E>
    where
        F: Future<Output = Result<V, E>> + Send + 'static,
    {
        let quote = {
            let mut quotes = self.quotes.lock().unwrap_or_else(PoisonError::into_inner);
            quotes
                .entry(key.clone())
                .or_insert_with(|| ask().boxed().shared())
                .clone()
        };
        let answer = quote.clone().await;
        if answer.is_err() {
            let mut quotes = self.quotes.lock().unwrap_or_else(PoisonError::into_inner);
            if quotes.get(&key).is_some_and(|kept| kept.ptr_eq(&quote)) {
                quotes.remove(&key);
            }
        }
        answer
    }
}
