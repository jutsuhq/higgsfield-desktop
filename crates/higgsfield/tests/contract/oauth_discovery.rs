//! `OAuthDiscoveryTests` and the network half of `OAuthHardeningTests`: metadata discovery, its validation,
//! issuer pinning and revocation.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::OAuthError;
use higgsfield::mcp::oauth::{DiscoveredAuthorization, OAuthDiscovery, ProtectedResourceMetadata, TokenSet};
use serde_json::Value;
use url::Url;

use crate::support::mcp::{Reply, fixture};
use crate::support::stub::{Stub, body_text, target};

const UNREACHABLE: &str = "https://unreachable.example";

/// The servers as recorded: resource metadata, clerk on RFC 8414, and the device server's 404s.
async fn server(resource: String, clerk: String, missing: Reply) -> Stub {
    Stub::replying(move |request| match target(request).as_str() {
        fixture::RESOURCE_METADATA_URL => Reply::json_text(&resource),
        url if url == format!("{}/.well-known/oauth-authorization-server", fixture::CLERK) => {
            Reply::json_text(&clerk)
        }
        _ => missing.clone(),
    })
    .await
}

async fn recorded(resource: String) -> Stub {
    let missing = Reply::json_text(fixture::NOT_FOUND_BODY).with_status(404);
    server(resource, fixture::CLERK_METADATA.to_owned(), missing).await
}

/// The recorded resource document with `authorization_servers` in place of the advertised ones.
fn resource(authorization_servers: &[&str]) -> String {
    let mut object: Value = serde_json::from_str(fixture::RESOURCE_METADATA).unwrap();
    object["authorization_servers"] = authorization_servers
        .iter()
        .map(|server| Value::from(*server))
        .collect();
    serde_json::to_string(&object).unwrap()
}

fn issuers(discovered: &DiscoveredAuthorization) -> Vec<String> {
    discovered
        .servers
        .iter()
        .map(|server| server.issuer.to_string())
        .collect()
}

fn discovery(stub: &Stub) -> OAuthDiscovery {
    OAuthDiscovery::new(stub.client())
}

/// `Url` writes a bare origin with a trailing slash.
fn origin(text: &str) -> String {
    format!("{text}/")
}

#[tokio::test]
async fn reads_recorded_metadata() {
    let stub = recorded(fixture::RESOURCE_METADATA.to_owned()).await;
    let discovered = discovery(&stub)
        .discover(&fixture::mcp_url(), Some(fixture::WWW_AUTHENTICATE))
        .await
        .unwrap();

    assert_eq!(
        target(&stub.requests().await[0]).as_str(),
        fixture::RESOURCE_METADATA_URL
    );
    let servers: Vec<_> = discovered
        .resource
        .authorization_servers
        .iter()
        .map(Url::to_string)
        .collect();
    assert_eq!(servers, [origin(fixture::CLERK), origin(fixture::DEVICE_AUTH)]);
    assert_eq!(
        discovered.resource.scopes_supported,
        Some(vec!["openid".into(), "email".into(), "offline_access".into()])
    );
    assert_eq!(issuers(&discovered), [origin(fixture::CLERK)]);

    let pkce = discovered.authorization_code_server().unwrap();
    let clerk = |path: &str| Url::parse(&format!("{}{path}", fixture::CLERK)).unwrap();
    assert_eq!(pkce.authorization_endpoint, Some(clerk("/oauth/authorize")));
    assert_eq!(pkce.token_endpoint, clerk("/oauth/token"));
    assert_eq!(pkce.registration_endpoint, Some(clerk("/oauth/register")));
    assert!(pkce.client_id_metadata_document_supported);
    assert!(pkce.authorization_response_iss_parameter_supported);
}

#[tokio::test]
async fn device_server_without_metadata_is_left_out() {
    let stub = recorded(fixture::RESOURCE_METADATA.to_owned()).await;
    let discovered = discovery(&stub)
        .discover(&fixture::mcp_url(), None)
        .await
        .unwrap();

    let device = fixture::DEVICE_AUTH;
    assert!(
        !stub
            .requests_to(&format!("{device}/.well-known/oauth-authorization-server"))
            .await
            .is_empty()
    );
    assert!(
        !stub
            .requests_to(&format!("{device}/.well-known/openid-configuration"))
            .await
            .is_empty()
    );
    assert_eq!(
        discovered.authorization_code_server().unwrap().issuer.to_string(),
        origin(fixture::CLERK)
    );
}

#[tokio::test]
async fn unreachable_server_does_not_hide_one_that_answers() {
    let stub = recorded(resource(&[UNREACHABLE, fixture::CLERK])).await;
    let lookups = Arc::new(AtomicUsize::new(0));
    let http = stub.client_with_unreachable("unreachable.example", lookups.clone());
    let discovered = OAuthDiscovery::new(http)
        .discover(&fixture::mcp_url(), None)
        .await
        .unwrap();

    assert_eq!(lookups.load(Ordering::SeqCst), 2);
    assert_eq!(issuers(&discovered), [origin(fixture::CLERK)]);
}

#[tokio::test]
async fn no_reachable_server_is_a_network_failure() {
    let stub = recorded(resource(&[UNREACHABLE])).await;
    let http = stub.client_with_unreachable("unreachable.example", Arc::default());
    let discovery = OAuthDiscovery::new(http);
    assert_eq!(
        discovery.discover(&fixture::mcp_url(), None).await,
        Err(HiggsfieldError::Network.into())
    );
    let mcp = Url::parse(&format!("{UNREACHABLE}/mcp")).unwrap();
    assert_eq!(
        discovery.discover(&mcp, None).await,
        Err(HiggsfieldError::Network.into())
    );
}

#[tokio::test]
async fn falls_back_to_root_resource_metadata() {
    let stub = Stub::replying(|request| match target(request).as_str() {
        "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource" => {
            Reply::json_text(fixture::RESOURCE_METADATA)
        }
        "https://clerk.higgsfield.ai/.well-known/oauth-authorization-server" => {
            Reply::json_text(fixture::CLERK_METADATA)
        }
        _ => Reply::not_found(),
    })
    .await;
    let discovered = discovery(&stub)
        .discover(&fixture::mcp_url(), None)
        .await
        .unwrap();
    assert_eq!(issuers(&discovered), [origin(fixture::CLERK)]);
}

#[tokio::test]
async fn fails_without_metadata() {
    let stub = Stub::replying(|_| Reply::not_found()).await;
    assert_eq!(
        discovery(&stub).discover(&fixture::mcp_url(), None).await,
        Err(OAuthError::DiscoveryFailed.into())
    );
}

// Issuer pinning: refresh goes only to the server that issued the tokens.

fn pinned() -> DiscoveredAuthorization {
    let resource: ProtectedResourceMetadata = serde_json::from_str(fixture::RESOURCE_METADATA).unwrap();
    DiscoveredAuthorization {
        resource,
        servers: vec![fixture::server(fixture::MCP_HOST_METADATA)],
    }
}

fn issued_by(issuer: Option<&str>) -> TokenSet {
    let mut tokens = fixture::tokens("access-1", Some("refresh-1"), fixture::distant_past());
    tokens.issuer = issuer.map(|issuer| Url::parse(issuer).unwrap());
    tokens
}

#[test]
fn refresh_uses_the_issuing_server() {
    let tokens = issued_by(Some("https://mcp.higgsfield.ai/"));
    let client = pinned().client_issuing(&tokens, reqwest::Client::new()).unwrap();
    assert_eq!(
        client.server.token_endpoint.as_str(),
        "https://mcp.higgsfield.ai/oauth2/token"
    );
    assert_eq!(client.client_id, "client-1");
    assert_eq!(client.resource, fixture::mcp_url());
}

#[test]
fn tokens_of_an_unlisted_issuer_need_a_new_sign_in() {
    // The MCP host is the only server: clerk's tokens must not go to it, even though it can refresh.
    let discovered = pinned();
    let reauth = Some(OAuthError::ReauthenticationRequired.into());
    assert_eq!(
        discovered
            .client_issuing(&issued_by(Some(fixture::CLERK)), reqwest::Client::new())
            .err(),
        reauth
    );
    assert_eq!(
        discovered
            .client_issuing(&issued_by(None), reqwest::Client::new())
            .err(),
        reauth
    );
}

// Discovery validation (RFC 9728 §3.3, RFC 8414 §3.3, https only).

async fn validating(resource: String, clerk: String) -> Stub {
    server(resource, clerk, Reply::not_found()).await
}

#[tokio::test]
async fn resource_for_another_url_fails_discovery() {
    let foreign = fixture::RESOURCE_METADATA.replace(
        r#""resource":"https://mcp.higgsfield.ai/mcp""#,
        r#""resource":"https://evil.example/mcp""#,
    );
    let stub = validating(foreign, fixture::CLERK_METADATA.to_owned()).await;
    assert_eq!(
        discovery(&stub).discover(&fixture::mcp_url(), None).await,
        Err(OAuthError::DiscoveryFailed.into())
    );
    let clerk = format!("{}/.well-known/oauth-authorization-server", fixture::CLERK);
    assert!(stub.requests_to(&clerk).await.is_empty());
}

#[tokio::test]
async fn server_claiming_another_issuer_is_left_out() {
    let mixed_up = fixture::CLERK_METADATA.replace(
        r#""issuer":"https://clerk.higgsfield.ai""#,
        r#""issuer":"https://evil.example""#,
    );
    let stub = validating(fixture::RESOURCE_METADATA.to_owned(), mixed_up).await;
    assert_eq!(
        discovery(&stub).discover(&fixture::mcp_url(), None).await,
        Err(OAuthError::DiscoveryFailed.into())
    );
}

#[tokio::test]
async fn plain_http_authorization_server_is_never_fetched() {
    let downgraded = fixture::RESOURCE_METADATA.replace(
        r#""authorization_servers":["https://clerk.higgsfield.ai""#,
        r#""authorization_servers":["http://clerk.higgsfield.ai""#,
    );
    let stub = validating(downgraded, fixture::CLERK_METADATA.to_owned()).await;
    assert_eq!(
        discovery(&stub).discover(&fixture::mcp_url(), None).await,
        Err(OAuthError::DiscoveryFailed.into())
    );
    assert!(
        !stub
            .requests()
            .await
            .iter()
            .any(|request| target(request).host_str() == Some("clerk.higgsfield.ai"))
    );
}

#[tokio::test]
async fn revoke_sends_the_refresh_token_to_the_revocation_endpoint() {
    let stub = Stub::replying(|_| Reply::status(200)).await;
    let clerk = fixture::server(fixture::CLERK_METADATA);
    let client = fixture::client(clerk, stub.client(), fixture::distant_past());
    client
        .revoke(&fixture::tokens(
            "access-1",
            Some("refresh-1"),
            fixture::distant_past(),
        ))
        .await
        .unwrap();

    let request = &stub.requests().await[0];
    assert_eq!(
        target(request).as_str(),
        format!("{}/oauth/token/revoke", fixture::CLERK)
    );
    assert_eq!(
        body_text(request),
        "client_id=client-1&token=refresh-1&token_type_hint=refresh_token"
    );
}
