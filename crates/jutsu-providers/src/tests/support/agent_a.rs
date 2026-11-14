//! Helpers of the tests agent a ported: a stub host answering by method and path (the Swift `StubServer.on`), an
//! account provider on a stub MCP server (`Provider.provider`), past generations, and the snapshot's image list.

use higgsfield::mcp::account::{AccountImageModel, MediaRole, PastGeneration, PastMedia};
use serde_json::Value;
use time::OffsetDateTime;
use url::Url;

use super::mcp::{McpServer, Reply};
use super::provider::make_provider;
use super::stub::Stub;
use crate::HiggsfieldMcpProvider;

/// `text` as a URL.
pub fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// A TLS stub answering each `(method, path, reply)` with its reply, and anything else with 404, like the Swift
/// `StubServer` with its `on(_:_:_:)` routes.
pub async fn routes(routes: Vec<(&'static str, String, Reply)>) -> Stub {
    Stub::replying(move |request| {
        routes
            .iter()
            .find(|(method, path, _)| request.method.as_str() == *method && request.url.path() == path)
            .map_or_else(Reply::not_found, |(_, _, reply)| reply.clone())
    })
    .await
}

/// An account provider on a stub MCP server listing `tools`, whose `tools/call` answers come from `call`
/// (`Provider.provider(tools:call:)`).
pub async fn mcp_provider(
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(tools).call(call).serve().await;
    (make_provider(&stub, false), stub)
}

/// A past generation with nothing but these, made at the epoch.
pub fn past_generation(
    id: &str,
    kind: &str,
    status: &str,
    model: &str,
    output: Option<&str>,
) -> PastGeneration {
    PastGeneration {
        id: id.to_owned(),
        kind: kind.to_owned(),
        status: status.to_owned(),
        model: model.to_owned(),
        created_at: OffsetDateTime::UNIX_EPOCH,
        is_liked: false,
        output_url: output.map(url),
        width: None,
        height: None,
        duration: None,
        prompt: String::new(),
        resolution: None,
        media: Vec::new(),
        is_draft: false,
        draft_job_id: None,
        mode: None,
    }
}

pub fn media(role: MediaRole, id: &str, link: Option<&str>) -> PastMedia {
    PastMedia {
        role: Some(role),
        id: id.to_owned(),
        url: link.map(url),
    }
}

/// `AccountImageModel(id:name:aspectRatios:imageRole:maxImages:)`.
pub fn image_model(
    id: &str,
    name: &str,
    aspect_ratios: &[&str],
    image_role: Option<&str>,
    max_images: Option<i64>,
) -> AccountImageModel {
    AccountImageModel {
        id: id.to_owned(),
        name: name.to_owned(),
        aspect_ratios: aspect_ratios.iter().map(|ratio| (*ratio).to_owned()).collect(),
        image_role: image_role.map(str::to_owned),
        max_images,
    }
}

/// The whole `models_explore {action: "list", type: "image"}` answer of the 2026-11-10 snapshot
/// (`CatalogSnapshot.imageListing()`).
pub fn image_listing() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mcp-snapshots/2026-11-10.json"
    ))
    .unwrap();
    let snapshot: Value = serde_json::from_str(&text).unwrap();
    snapshot["models"]["list"]["image"][0]["structuredContent"].clone()
}
