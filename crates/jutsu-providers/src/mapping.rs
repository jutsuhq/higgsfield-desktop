//! The client crate's errors and states in the engine's terms.

use higgsfield::core::{GenerationStatus, HiggsfieldError, ImageStatus, Moderation, is_https};
use higgsfield::mcp::oauth::TokenSet;
use higgsfield::mcp::{McpError, OAuthError};
use jutsu_core::{
    Cost, ImageJobStatus, OAuthTokens, PortError, PricingError, ProviderError, Rejection, RemoteStatus,
    SignInError, decimal_shortest_of,
};
use rust_decimal::Decimal;

/// The message of a completed job whose result isn't on HTTPS.
pub(crate) const INSECURE_RESULT: &str = "The result isn't on a secure (https) address.";

/// A refused request, as the engine's validation failure.
pub(crate) fn validation(field: Option<&str>, message: impl Into<String>) -> PortError {
    PortError::Provider(ProviderError::Validation {
        field: field.map(str::to_owned),
        message: message.into(),
    })
}

/// A failure of the engine's own kind.
pub(crate) fn provider(error: ProviderError) -> PortError {
    PortError::Provider(error)
}

/// A client failure in the engine's terms. `Cancelled` stays a cancellation, which the engine never retries.
pub(crate) fn port(error: HiggsfieldError) -> PortError {
    match error {
        HiggsfieldError::Cancelled => PortError::Cancelled,
        error => PortError::Provider(provider_error(error)),
    }
}

/// `ProviderError.init(_: HiggsfieldError)`. A cancelled call has no provider error; it reads as a network failure
/// here, and `port` keeps it a cancellation.
pub fn provider_error(error: HiggsfieldError) -> ProviderError {
    match error {
        HiggsfieldError::Unauthorized => ProviderError::Unauthorized,
        HiggsfieldError::InsufficientFunds => ProviderError::InsufficientFunds,
        HiggsfieldError::ConcurrencyLimit { retry_after } => ProviderError::ConcurrencyLimit {
            retry_after: retry_after.and_then(time::Duration::checked_seconds_f64),
        },
        HiggsfieldError::Validation { field, message } => ProviderError::Validation { field, message },
        HiggsfieldError::ModelUnavailable => ProviderError::ModelUnavailable,
        HiggsfieldError::ToolUnavailable(_) => not_available(),
        HiggsfieldError::Server { status } => ProviderError::Server {
            status: i64::from(status),
        },
        HiggsfieldError::Network | HiggsfieldError::NotSent | HiggsfieldError::Cancelled => {
            ProviderError::Network
        }
    }
}

/// What the account connection says of a tool or job it doesn't offer.
pub(crate) fn not_available() -> ProviderError {
    ProviderError::Validation {
        field: None,
        message: "Not available for this account".to_owned(),
    }
}

/// The bundled prices failed to load; Swift rethrows its own error, which has no provider meaning.
pub(crate) fn pricing(error: &PricingError) -> PortError {
    PortError::Other(error.to_string())
}

/// Server amounts are untrusted; a malformed quote must never look free or overflow variant pricing.
pub(crate) fn server_credits(value: f64, is_estimate: bool, count: i64) -> Result<Cost, PortError> {
    let amount = (0.0..1e12)
        .contains(&value)
        .then(|| decimal_shortest_of(value))
        .flatten()
        .ok_or(PortError::Provider(ProviderError::Server { status: 200 }))?;
    Ok(Cost::credits(amount * Decimal::from(count), is_estimate))
}

/// `RemoteStatus.init(_: GenerationStatus)`: a result that isn't on HTTPS fails.
pub fn remote_status(status: GenerationStatus) -> RemoteStatus {
    match status {
        GenerationStatus::Queued => RemoteStatus::Queued,
        GenerationStatus::InProgress => RemoteStatus::InProgress,
        GenerationStatus::Completed { video_url } if is_https(&video_url) => {
            RemoteStatus::Completed { video_url }
        }
        GenerationStatus::Completed { .. } => RemoteStatus::Failed {
            message: Some(INSECURE_RESULT.to_owned()),
        },
        GenerationStatus::Failed { message } => RemoteStatus::Failed { message },
        GenerationStatus::Rejected(moderation) => RemoteStatus::Rejected(rejection(moderation)),
        GenerationStatus::Canceled => RemoteStatus::Canceled,
    }
}

/// `ImageJobStatus.init(_: ImageStatus)`.
pub fn image_status(status: ImageStatus) -> ImageJobStatus {
    match status {
        ImageStatus::Queued | ImageStatus::InProgress => ImageJobStatus::Pending,
        ImageStatus::Completed { image_urls } => ImageJobStatus::Completed(image_urls),
        ImageStatus::Failed { message } => ImageJobStatus::Failed { message },
        ImageStatus::Rejected(moderation) => ImageJobStatus::Rejected(rejection(moderation)),
        ImageStatus::Canceled => ImageJobStatus::Canceled,
    }
}

/// `Rejection.init(_: Moderation)`.
pub fn rejection(moderation: Moderation) -> Rejection {
    match moderation {
        Moderation::ExplicitContent => Rejection::ExplicitContent,
        Moderation::ProtectedContent => Rejection::ProtectedContent,
    }
}

/// `AccountSignIn.error(for:)`: a failed account sign-in as the `SignInError` the user is shown.
pub fn sign_in_error(error: OAuthError) -> SignInError {
    match error {
        OAuthError::DiscoveryFailed => SignInError::Unavailable,
        OAuthError::UnsupportedFlow
        | OAuthError::RegistrationUnavailable
        | OAuthError::RedirectUnavailable => SignInError::FlowUnavailable,
        OAuthError::StateMismatch | OAuthError::IssuerMismatch | OAuthError::InvalidCallback => {
            SignInError::InvalidResponse
        }
        OAuthError::AccessDenied => SignInError::Denied,
        OAuthError::TimedOut => SignInError::TimedOut,
        OAuthError::ReauthenticationRequired => SignInError::ReauthenticationRequired,
        OAuthError::Server { code, description } => SignInError::Server(Some(description.unwrap_or(code))),
    }
}

/// `AccountSignIn.error(for:)` of any failed sign-in call: an OAuth failure as the `SignInError` the user is
/// shown, a client failure as the engine's, anything else as its description.
pub fn account_sign_in_error(error: McpError) -> PortError {
    match error {
        McpError::OAuth(error) => PortError::SignIn(sign_in_error(error)),
        McpError::Higgsfield(error) => port(error),
        error => PortError::Other(error.to_string()),
    }
}

/// The stored credential of the SDK's token set (`OAuthTokens.init(_: TokenSet)`).
pub fn oauth_tokens(tokens: TokenSet) -> OAuthTokens {
    OAuthTokens {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens.expires_at,
        scope: tokens.scope,
        client_id: tokens.client_id,
        issuer: tokens.issuer,
        email: tokens.email,
        name: tokens.name,
        subject: tokens.subject,
    }
}

/// The SDK's token set of a stored credential (`OAuthTokens.tokenSet`).
pub fn token_set(tokens: OAuthTokens) -> TokenSet {
    let mut set = TokenSet::new(
        tokens.access_token,
        tokens.refresh_token,
        tokens.expires_at,
        tokens.scope,
        tokens.client_id,
    );
    set.issuer = tokens.issuer;
    set.email = tokens.email;
    set.name = tokens.name;
    set.subject = tokens.subject;
    set
}
