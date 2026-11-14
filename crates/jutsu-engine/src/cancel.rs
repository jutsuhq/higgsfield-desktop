//! The cancellation of the task a step runs in, as Swift's `Task.isCancelled` sees it.
//!
//! Each job's task runs inside `scope` with its own `CancellationToken`. Code it calls checks `is_cancelled` where
//! the Swift original calls `Task.checkCancellation()`, and every wait on a port or a timer goes through
//! `cancellable`, so cancelling the token cuts the wait off (the future is dropped). Work outside a job's task (a
//! public call, an enqueue) has no token and is never cancelled, like Swift's unstructured tasks.

use std::future::Future;

use tokio_util::sync::CancellationToken;

tokio::task_local! {
    static TOKEN: CancellationToken;
}

/// The wait was cut off because its task was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cancelled;

/// Runs `future` as a task cancelled by `token`.
pub(crate) fn scope<F: Future>(token: CancellationToken, future: F) -> impl Future<Output = F::Output> {
    TOKEN.scope(token, future)
}

/// Whether the current task was cancelled.
pub(crate) fn is_cancelled() -> bool {
    TOKEN.try_with(CancellationToken::is_cancelled).unwrap_or(false)
}

/// `future`'s output, or `Cancelled` as soon as the current task is cancelled (at once when it already is).
pub(crate) async fn cancellable<F: Future>(future: F) -> Result<F::Output, Cancelled> {
    let Ok(token) = TOKEN.try_with(Clone::clone) else {
        return Ok(future.await);
    };
    tokio::select! {
        biased;
        () = token.cancelled() => Err(Cancelled),
        output = future => Ok(output),
    }
}

/// As `cancellable`, but an output that is there when the task is cancelled is kept rather than dropped, as Swift's
/// awaited call that returns while its task is being cancelled still returns its value.
pub(crate) async fn answered<F: Future>(future: F) -> Result<F::Output, Cancelled> {
    let Ok(token) = TOKEN.try_with(Clone::clone) else {
        return Ok(future.await);
    };
    tokio::select! {
        biased;
        output = future => Ok(output),
        () = token.cancelled() => Err(Cancelled),
    }
}
