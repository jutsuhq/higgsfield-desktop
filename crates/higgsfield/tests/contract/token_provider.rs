//! `TokenProviderTests`: refresh ahead of expiry, one shared refresh, and how refresh failures map.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::oauth::{TokenProvider, TokenSet};
use higgsfield::mcp::{McpError, OAuthError};
use time::OffsetDateTime;

use crate::support::mcp::{DEFAULT_BEARER_CHALLENGE, Locked, Reply, fixture};
use crate::support::stub::Stub;

struct Setup {
    now: OffsetDateTime,
    refreshes: Arc<Locked<usize>>,
    persisted: Arc<Locked<Vec<TokenSet>>>,
}

fn setup() -> Setup {
    Setup {
        now: OffsetDateTime::from_unix_timestamp(10_000).unwrap(),
        refreshes: Locked::new(0),
        persisted: Locked::new(Vec::new()),
    }
}

fn invalid_grant() -> McpError {
    OAuthError::Server {
        code: "invalid_grant".into(),
        description: None,
    }
    .into()
}

impl Setup {
    /// `failures`: how many refreshes fail before one succeeds.
    fn provider(&self, expires_in: i64, refresh_token: Option<&str>, failures: usize) -> TokenProvider {
        let now = self.now;
        let refreshes = self.refreshes.clone();
        let persisted = self.persisted.clone();
        TokenProvider::new(
            fixture::tokens(
                "access-1",
                refresh_token,
                now + time::Duration::seconds(expires_in),
            ),
            move |old: TokenSet| {
                let count = refreshes.update(|count| {
                    *count += 1;
                    *count
                });
                async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if count <= failures {
                        return Err(invalid_grant());
                    }
                    let access = format!("access-{}", count + 1);
                    Ok(fixture::tokens(
                        &access,
                        old.refresh_token.as_deref(),
                        now + time::Duration::hours(1),
                    ))
                }
            },
            move |tokens| {
                persisted.update(|list| list.push(tokens));
                async {}
            },
        )
        .with_clock(Arc::new(move || now))
    }

    /// A provider whose refresh fails once with `error`, then succeeds.
    fn failing_once_with(&self, error: McpError) -> TokenProvider {
        let now = self.now;
        let refreshes = self.refreshes.clone();
        TokenProvider::new(
            fixture::tokens("access-1", Some("refresh-1"), now),
            move |old: TokenSet| {
                let count = refreshes.update(|count| {
                    *count += 1;
                    *count
                });
                let error = error.clone();
                async move {
                    if count == 1 {
                        return Err(error);
                    }
                    let access = format!("access-{}", count + 1);
                    Ok(fixture::tokens(
                        &access,
                        old.refresh_token.as_deref(),
                        now + time::Duration::hours(1),
                    ))
                }
            },
            |_| async {},
        )
        .with_clock(Arc::new(move || now))
    }
}

async fn ten_calls(provider: &TokenProvider) -> Vec<Result<String, McpError>> {
    let calls = (0..10).map(|_| {
        let provider = provider.clone();
        tokio::spawn(async move { provider.access_token().await })
    });
    futures::future::join_all(calls)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect()
}

#[tokio::test]
async fn valid_token_is_returned_without_refresh() {
    let setup = setup();
    assert_eq!(
        setup
            .provider(61, Some("refresh-1"), 0)
            .access_token()
            .await
            .unwrap(),
        "access-1"
    );
    assert_eq!(setup.refreshes.value(), 0);
}

#[tokio::test]
async fn token_inside_the_margin_is_refreshed_and_persisted() {
    let setup = setup();
    assert_eq!(
        setup
            .provider(59, Some("refresh-1"), 0)
            .access_token()
            .await
            .unwrap(),
        "access-2"
    );
    assert_eq!(setup.refreshes.value(), 1);
    let persisted: Vec<_> = setup
        .persisted
        .value()
        .into_iter()
        .map(|tokens| tokens.access_token)
        .collect();
    assert_eq!(persisted, ["access-2"]);
}

#[tokio::test]
async fn concurrent_callers_share_one_refresh() {
    let setup = setup();
    let provider = setup.provider(-10, Some("refresh-1"), 0);
    let tokens: HashSet<_> = ten_calls(&provider)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(setup.refreshes.value(), 1);
    assert_eq!(tokens, HashSet::from(["access-2".to_owned()]));
}

#[tokio::test]
async fn failed_refresh_is_shared_then_retried() {
    let setup = setup();
    let provider = setup.provider(-10, Some("refresh-1"), 1);
    assert!(ten_calls(&provider).await.iter().all(Result::is_err));
    assert_eq!(setup.refreshes.value(), 1);

    let tokens: HashSet<_> = ten_calls(&provider)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(tokens, HashSet::from(["access-3".to_owned()]));
    assert_eq!(provider.access_token().await.unwrap(), "access-3");
    assert_eq!(setup.refreshes.value(), 2);
    let persisted: Vec<_> = setup
        .persisted
        .value()
        .into_iter()
        .map(|tokens| tokens.access_token)
        .collect();
    assert_eq!(persisted, ["access-3"]);
}

#[tokio::test]
async fn rejected_token_refreshes_only_once() {
    let setup = setup();
    let provider = setup.provider(3600, Some("refresh-1"), 0);
    let (first, second) = tokio::join!(
        provider.access_token_replacing("access-1"),
        provider.access_token_replacing("access-1")
    );
    assert_eq!([first.unwrap(), second.unwrap()], ["access-2", "access-2"]);
    assert_eq!(
        provider.access_token_replacing("access-1").await.unwrap(),
        "access-2"
    );
    assert_eq!(setup.refreshes.value(), 1);
}

#[tokio::test]
async fn refused_refresh_requires_sign_in() {
    let setup = setup();
    let reauth = Err(OAuthError::ReauthenticationRequired.into());
    assert_eq!(
        setup.provider(0, Some("refresh-1"), 1).access_token().await,
        reauth
    );
    assert_eq!(setup.provider(0, None, 0).access_token().await, reauth);
    assert_eq!(setup.refreshes.value(), 1);
    assert_eq!(setup.persisted.value().len(), 0);
}

#[tokio::test]
async fn server_error_keeps_the_tokens_for_the_next_call() {
    let provider = setup().failing_once_with(HiggsfieldError::Server { status: 503 }.into());
    assert_eq!(
        provider.access_token().await,
        Err(HiggsfieldError::Server { status: 503 }.into())
    );
    assert_eq!(provider.access_token().await.unwrap(), "access-3");
}

/// Unreadable metadata, an unreadable reply or another OAuth error: no new sign-in, and the next call refreshes.
#[tokio::test]
async fn other_refresh_failures_are_network_failures_that_keep_the_tokens() {
    let errors: [McpError; 3] = [
        OAuthError::DiscoveryFailed.into(),
        McpError::Decoding("not JSON".into()),
        OAuthError::Server {
            code: "temporarily_unavailable".into(),
            description: None,
        }
        .into(),
    ];
    for error in errors {
        let provider = setup().failing_once_with(error.clone());
        assert_eq!(
            provider.access_token().await,
            Err(HiggsfieldError::Network.into()),
            "{error:?}"
        );
        assert_eq!(provider.access_token().await.unwrap(), "access-3");
    }
}

/// `OAuthClient::refresh` against the token endpoint, as `TokenProvider` sees it.
async fn refresh(answer: Reply) -> Result<String, McpError> {
    let stub = Stub::replying(move |_| answer.clone()).await;
    let client = Arc::new(fixture::client(
        fixture::server(fixture::CLERK_METADATA),
        stub.client(),
        fixture::distant_past(),
    ));
    let provider = TokenProvider::new(
        fixture::tokens("access-1", Some("refresh-1"), fixture::distant_past()),
        move |tokens: TokenSet| {
            let client = client.clone();
            async move { client.refresh(&tokens).await }
        },
        |_| async {},
    );
    provider.access_token().await
}

#[tokio::test]
async fn outage_is_a_server_error() {
    let unavailable = Reply::json_text(r#"{"error":"temporarily_unavailable"}"#).with_status(503);
    assert_eq!(
        refresh(unavailable).await,
        Err(HiggsfieldError::Server { status: 503 }.into())
    );
    let gateway = Reply::status(502).body("<html>Bad Gateway</html>");
    assert_eq!(
        refresh(gateway).await,
        Err(HiggsfieldError::Server { status: 502 }.into())
    );
}

#[tokio::test]
async fn unreadable_token_body_is_a_server_error() {
    let answer = Reply::json_text(r#"{"token":"no access_token here"}"#);
    assert_eq!(
        refresh(answer).await,
        Err(HiggsfieldError::Server { status: 200 }.into())
    );
}

#[tokio::test]
async fn refused_grant_requires_sign_in() {
    let reauth = Err(OAuthError::ReauthenticationRequired.into());
    assert_eq!(
        refresh(Reply::json_text(r#"{"error":"invalid_grant"}"#).with_status(400)).await,
        reauth
    );
    let refused_client = Reply::json_text(r#"{"error":"invalid_client"}"#)
        .with_status(401)
        .header("WWW-Authenticate", DEFAULT_BEARER_CHALLENGE);
    assert_eq!(refresh(refused_client).await, reauth);
    assert_eq!(refresh(Reply::status(400)).await, reauth);
}
