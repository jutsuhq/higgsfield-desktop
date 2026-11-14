//! Port of `AccountSignInTests.swift`.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::{McpError, OAuthError};
use jutsu_core::{PortError, ProviderError, SignInError};

use crate::mapping::{account_sign_in_error, sign_in_error};

#[test]
fn oauth_failures_become_sign_in_errors() {
    let cases = [
        (OAuthError::DiscoveryFailed, SignInError::Unavailable),
        (OAuthError::UnsupportedFlow, SignInError::FlowUnavailable),
        (OAuthError::RegistrationUnavailable, SignInError::FlowUnavailable),
        (OAuthError::RedirectUnavailable, SignInError::FlowUnavailable),
        (OAuthError::StateMismatch, SignInError::InvalidResponse),
        (OAuthError::IssuerMismatch, SignInError::InvalidResponse),
        (OAuthError::InvalidCallback, SignInError::InvalidResponse),
        (OAuthError::AccessDenied, SignInError::Denied),
        (OAuthError::TimedOut, SignInError::TimedOut),
        (
            OAuthError::ReauthenticationRequired,
            SignInError::ReauthenticationRequired,
        ),
        (
            OAuthError::Server {
                code: "invalid_client".to_owned(),
                description: Some("Client not allowed".to_owned()),
            },
            SignInError::Server(Some("Client not allowed".to_owned())),
        ),
        (
            OAuthError::Server {
                code: "invalid_client".to_owned(),
                description: None,
            },
            SignInError::Server(Some("invalid_client".to_owned())),
        ),
    ];
    for (oauth, expected) in cases {
        assert_eq!(sign_in_error(oauth.clone()), expected, "{oauth:?}");
        assert_eq!(
            account_sign_in_error(McpError::OAuth(oauth.clone())),
            PortError::SignIn(expected),
            "{oauth:?}"
        );
    }
}

#[test]
fn other_errors_pass_through() {
    assert_eq!(
        account_sign_in_error(McpError::Higgsfield(HiggsfieldError::Cancelled)),
        PortError::Cancelled
    );
    assert_eq!(
        account_sign_in_error(McpError::Higgsfield(HiggsfieldError::Network)),
        PortError::Provider(ProviderError::Network)
    );
}
