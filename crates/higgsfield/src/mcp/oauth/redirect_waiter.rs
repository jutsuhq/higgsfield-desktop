use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::authorizer::state_of;
use crate::core::HiggsfieldError;
use crate::mcp::error::{McpError, OAuthError};

/// Waits for the one authorization redirect of a sign-in. Any web page or local process can reach a loopback
/// port, so only a callback carrying the authorization request's `state` counts; others are ignored and waiting
/// goes on. Gives up with `OAuthError::TimedOut` once the deadline passes.
#[derive(Debug, Default)]
pub(crate) struct RedirectWaiter {
    state: Mutex<State>,
    delivered: Notify,
}

/// Which callbacks count.
#[derive(Debug, Default)]
enum Expected {
    /// Before `expect`: callbacks are ignored.
    #[default]
    Nothing,
    /// A request sent without `state`: any callback.
    Any,
    State(String),
}

#[derive(Debug, Default)]
struct State {
    expected: Expected,
    received: Option<Result<Url, McpError>>,
}

impl RedirectWaiter {
    fn locked(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts accepting callbacks with `state`. `None` accepts any callback, for a request sent without one.
    pub fn expect(&self, state: Option<String>) {
        self.locked().expected = state.map_or(Expected::Any, Expected::State);
    }

    /// Hands `callback` to the waiting sign-in when its `state` matches. Returns whether it did.
    pub fn offer(&self, callback: Url) -> bool {
        if !self.accepts(&callback) {
            return false;
        }
        self.deliver(Ok(callback));
        true
    }

    pub fn fail(&self, error: McpError) {
        self.deliver(Err(error));
    }

    /// The accepted callback. `OAuthError::TimedOut` after `timeout`, `HiggsfieldError::Cancelled` once `cancel`
    /// is cancelled.
    pub async fn wait(&self, timeout: Duration, cancel: &CancellationToken) -> Result<Url, McpError> {
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            let delivered = self.delivered.notified();
            tokio::pin!(delivered);
            delivered.as_mut().enable();
            if let Some(result) = self.locked().received.clone() {
                return result;
            }
            tokio::select! {
                () = &mut delivered => {}
                () = &mut deadline => self.fail(OAuthError::TimedOut.into()),
                () = cancel.cancelled() => self.fail(HiggsfieldError::Cancelled.into()),
            }
        }
    }

    /// Whether `offer` would take `callback`.
    pub fn accepts(&self, callback: &Url) -> bool {
        let state = state_of(callback);
        match &self.locked().expected {
            Expected::Nothing => false,
            Expected::Any => true,
            Expected::State(expected) => state.as_deref() == Some(expected.as_str()),
        }
    }

    /// Keeps the first result and wakes the waiting `wait`.
    fn deliver(&self, result: Result<Url, McpError>) {
        let mut state = self.locked();
        if state.received.is_none() {
            state.received = Some(result);
            drop(state);
            self.delivered.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callback(state: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:47613/callback?code=c&state={state}")).unwrap()
    }

    fn waiter() -> RedirectWaiter {
        let waiter = RedirectWaiter::default();
        waiter.expect(Some("right".into()));
        waiter
    }

    #[tokio::test]
    async fn callback_with_another_state_is_ignored_and_waiting_goes_on() {
        let waiter = waiter();
        let stateless = Url::parse("http://127.0.0.1:47613/callback").unwrap();
        assert!(!waiter.offer(callback("forged")));
        assert!(!waiter.offer(stateless));
        assert!(waiter.offer(callback("right")));
        let received = waiter
            .wait(Duration::from_secs(30), &CancellationToken::new())
            .await;
        assert_eq!(received, Ok(callback("right")));
    }

    #[test]
    fn callbacks_before_the_request_is_known_are_ignored() {
        assert!(!RedirectWaiter::default().offer(callback("right")));
    }

    #[tokio::test]
    async fn missing_redirect_fails_after_the_deadline() {
        let result = waiter()
            .wait(Duration::from_millis(50), &CancellationToken::new())
            .await;
        assert_eq!(result, Err(OAuthError::TimedOut.into()));
    }

    #[tokio::test]
    async fn cancelling_stops_waiting() {
        let waiter = std::sync::Arc::new(waiter());
        let cancel = CancellationToken::new();
        let waiting = tokio::spawn({
            let (waiter, cancel) = (waiter.clone(), cancel.clone());
            async move { waiter.wait(Duration::from_secs(30), &cancel).await }
        });
        cancel.cancel();
        assert_eq!(waiting.await.unwrap(), Err(HiggsfieldError::Cancelled.into()));
    }
}
