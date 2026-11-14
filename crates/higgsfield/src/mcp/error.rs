use serde::Deserialize;

use crate::core::HiggsfieldError;

/// A failure of the MCP client or the OAuth sign-in. `HiggsfieldAccount` maps every one to `HiggsfieldError`.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Higgsfield(#[from] HiggsfieldError),
    #[error(transparent)]
    OAuth(#[from] OAuthError),
    /// A JSON-RPC error object, as the server sent it.
    #[error(transparent)]
    JsonRpc(#[from] JsonRpcError),
    /// A 2xx answer whose JSON doesn't have the expected shape.
    #[error("unreadable answer: {0}")]
    Decoding(String),
    /// A request value that can't be written as JSON, such as a NaN number; nothing was sent.
    #[error("the request has a value that can't be sent")]
    Encoding,
    /// The browser could not be opened for the sign-in, with the opener's reason.
    #[error("the browser could not be opened: {0}")]
    Browser(String),
}

impl McpError {
    pub(crate) fn server(status: u16) -> Self {
        Self::Higgsfield(HiggsfieldError::Server { status })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum OAuthError {
    /// No protected resource or authorization server metadata could be read.
    #[error("no OAuth metadata could be read")]
    DiscoveryFailed,
    /// None of the advertised authorization servers supports the requested flow.
    #[error("no authorization server supports the sign-in flow")]
    UnsupportedFlow,
    /// The server neither accepts a client ID metadata document nor offers dynamic registration.
    #[error("the authorization server offers no client registration")]
    RegistrationUnavailable,
    /// The callback's `state` differs from the one sent: a forged or stale redirect.
    #[error("the redirect's state doesn't match")]
    StateMismatch,
    /// The callback's `iss` names another authorization server (RFC 9207 mix-up defense).
    #[error("the redirect names another issuer")]
    IssuerMismatch,
    #[error("the redirect has no code")]
    InvalidCallback,
    /// None of the loopback redirect ports could be opened, e.g. all taken by other apps.
    #[error("no loopback redirect port is free")]
    RedirectUnavailable,
    #[error("access was denied")]
    AccessDenied,
    /// The browser didn't come back with the redirect before the sign-in's deadline.
    #[error("the sign-in timed out")]
    TimedOut,
    /// The refresh token is missing or was refused; the user has to sign in again.
    #[error("sign in again")]
    ReauthenticationRequired,
    /// An OAuth error response (RFC 6749 §5.2).
    #[error("OAuth error {code}")]
    Server {
        code: String,
        description: Option<String>,
    },
}

impl OAuthError {
    /// RFC 6749 §5.2 codes that refuse the grant or the client, so the stored tokens can never be refreshed.
    pub const REFUSAL_CODES: [&'static str; 2] = ["invalid_grant", "invalid_client"];

    /// Whether this failure of a refresh means the user has to sign in again.
    #[must_use]
    pub fn refuses_refresh(&self) -> bool {
        match self {
            Self::ReauthenticationRequired => true,
            Self::Server { code, .. } => Self::REFUSAL_CODES.contains(&code.as_str()),
            _ => false,
        }
    }
}

/// A JSON-RPC 2.0 error object, returned as is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, thiserror::Error)]
#[error("JSON-RPC error {code}: {message}")]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}
