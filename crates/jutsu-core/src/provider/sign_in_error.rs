//! Sign-in failures.

/// Sign-in failures in terms the user can act on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum SignInError {
    /// No authorization server could be read.
    #[error("unavailable")]
    Unavailable,
    /// The server doesn't offer this flow, or won't register the app.
    #[error("flowUnavailable")]
    FlowUnavailable,
    /// The person said no.
    #[error("denied")]
    Denied,
    /// The browser didn't come back before the sign-in's deadline.
    #[error("timedOut")]
    TimedOut,
    /// A redirect or token answer that doesn't check out (state, issuer, missing code).
    #[error("invalidResponse")]
    InvalidResponse,
    /// The stored sign-in can't be refreshed any more.
    #[error("reauthenticationRequired")]
    ReauthenticationRequired,
    /// An OAuth error response; the description when the server sent one.
    #[error("server")]
    Server(Option<String>),
}
