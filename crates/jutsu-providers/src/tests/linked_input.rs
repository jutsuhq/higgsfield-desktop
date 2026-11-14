//! Port of `LinkedInputTests.swift`.

use jutsu_core::{GenerationProvider, MediaLinkKind, RemoteAsset};
use serde_json::{Value, json};

use super::support::account::{calls, full_set, schema_violations};
use super::support::agent_a::{mcp_provider, routes, url};
use super::support::catalog;
use super::support::provider::make_api_provider;
use crate::HiggsfieldMcpProvider;

const ORIGIN: &str = "https://media.example.org/clip.mp4";

#[tokio::test]
async fn an_api_key_sends_a_linked_file_by_its_link_up_to_200_mib() {
    let stub = routes(Vec::new()).await;
    let provider = make_api_provider(&stub, "key-id");

    let linked = provider
        .linked_asset(&url(ORIGIN), MediaLinkKind::Video, 5_000_000)
        .await
        .unwrap();
    let too_large = provider
        .linked_asset(&url(ORIGIN), MediaLinkKind::Video, 201 * 1024 * 1024)
        .await
        .unwrap();
    let insecure = url("http://media.example.org/clip.mp4");
    let not_https = provider
        .linked_asset(&insecure, MediaLinkKind::Video, 1)
        .await
        .unwrap();

    assert_eq!(linked, Some(RemoteAsset::new(url(ORIGIN))));
    assert_eq!(too_large, None);
    assert_eq!(not_https, None);
    assert!(stub.requests().await.is_empty());
}

// The account imports a linked file with `media_import_url` (the catalog snapshot of 2026-10-15's tool).

const MEDIA_ID: &str = "5b7d1e90-2a4c-4e8f-9d36-7c1a2b3e4f50";

async fn account_provider(
    reply: impl Fn(&Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, super::support::stub::Stub) {
    let mut tools = full_set();
    tools.push(catalog::tool("media_import_url"));
    mcp_provider(tools, move |name, arguments| {
        if name == "media_import_url" {
            reply(arguments)
        } else {
            json!({})
        }
    })
    .await
}

#[tokio::test]
async fn a_linked_file_up_to_50_mb_is_imported_by_its_link() {
    let (provider, stub) = account_provider(|_| json!({"media_id": MEDIA_ID, "type": "video"})).await;

    let linked = provider
        .linked_asset(&url(ORIGIN), MediaLinkKind::Video, 50_000_000)
        .await
        .unwrap();

    assert_eq!(
        linked,
        Some(RemoteAsset {
            url: url(ORIGIN),
            id: Some(MEDIA_ID.to_owned())
        })
    );
    assert_eq!(
        calls(&stub).await,
        [json!({
            "name": "media_import_url",
            "arguments": {"url": "https://media.example.org/clip.mp4", "type": "video"},
        })]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_large_or_insecure_file_is_uploaded_without_a_call() {
    let (provider, stub) = account_provider(|_| json!({"media_id": MEDIA_ID})).await;
    let insecure = url("http://media.example.org/clip.mp4");

    assert_eq!(
        provider
            .linked_asset(&url(ORIGIN), MediaLinkKind::Video, 50_000_001)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        provider
            .linked_asset(&insecure, MediaLinkKind::Image, 1000)
            .await
            .unwrap(),
        None
    );
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}

#[tokio::test]
async fn a_refused_import_or_a_missing_tool_uploads() {
    let (refusing, _refusing_stub) =
        account_provider(|_| json!({"error": "Remote host refused the download"})).await;
    assert_eq!(
        refusing
            .linked_asset(&url(ORIGIN), MediaLinkKind::Image, 1000)
            .await
            .unwrap(),
        None
    );

    let (without, stub) = mcp_provider(full_set(), |_, _| json!({})).await;
    assert_eq!(
        without
            .linked_asset(&url(ORIGIN), MediaLinkKind::Video, 1000)
            .await
            .unwrap(),
        None
    );
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}
