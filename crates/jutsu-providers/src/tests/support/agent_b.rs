//! Helpers of the tests agent b ported: the Swift `Network.Provider` suite's fixtures.

use std::str::FromStr;
use std::sync::Mutex;

use jutsu_core::{
    ClosedRange, Cost, JobSpec, LocalImage, Mode, PixelSize, RemoteAsset, RemoteInputs, Resolution,
    SourceClip, VideoModel,
};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use url::Url;
use wiremock::Request;

use super::account::calls_of;
use super::catalog;
use super::mcp::{McpServer, Reply, tool};
use super::provider::make_provider;
use super::stub::{Stub, body_json};
use crate::HiggsfieldMcpProvider;

/// `Provider.fullSet`: the tools as the live server gave them on 2026-10-14.
pub fn provider_tools() -> Vec<Value> {
    vec![
        tool("generate_image", "Text to image with motion blur", &["params"]),
        tool(
            "generate_video",
            "Generate one direct video request",
            &["params", "context"],
        ),
        tool(
            "media_upload",
            "Upload media for use in generation",
            &["filename", "content_type"],
        ),
        tool("media_confirm", "Confirm file uploads", &["media_id", "type"]),
        tool(
            "job_status",
            "Check the status and results of an async job",
            &["jobId", "sync"],
        ),
        tool(
            "balance",
            "Get the user's available credits and current plan",
            &[],
        ),
    ]
}

/// `Provider.provider(tools:call:)`: an account provider on an MCP stub listing `tools`, nobody opted in to unlimited.
pub async fn provider(
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(tools).call(call).serve().await;
    (make_provider(&stub, false), stub)
}

/// `provider` whose every call answers `{}`.
pub async fn quiet_provider(tools: Vec<Value>) -> (HiggsfieldMcpProvider, Stub) {
    provider(tools, |_, _| json!({})).await
}

pub fn file_url(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
}

/// `Provider.spec(mode)`: Genjutsu at 720p on the whole of a 10 s 1280×720 clip.
pub fn spec(mode: Mode) -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        10.0,
        PixelSize {
            width: 1280,
            height: 720,
        },
        ClosedRange::new(0.0, 10.0),
    );
    JobSpec::new(mode, VideoModel::GENJUTSU, Some(source), Resolution::P720)
}

pub fn asset(path: &str, id: &str) -> RemoteAsset {
    RemoteAsset {
        url: file_url(path),
        id: Some(id.to_owned()),
    }
}

/// `Provider.inputs`: the clip as `video-1` and one image as `image-1`.
pub fn inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(asset("/in.mp4", "video-1")),
        images: vec![asset("/a.jpg", "image-1")],
        audio: None,
    }
}

/// `Provider.image`.
pub fn image() -> LocalImage {
    LocalImage::new(file_url("/tmp/a.jpg"))
}

/// `amount` credits, written as decimal text.
pub fn credits(amount: &str, is_estimate: bool) -> Cost {
    Cost::credits(Decimal::from_str(amount).unwrap(), is_estimate)
}

/// Swift's `JSONValue` equality: numbers by value.
#[track_caller]
pub fn assert_same(actual: &Value, expected: &Value) {
    assert!(catalog::same(actual, expected), "{actual} is not {expected}");
}

/// The `params` of every `generate_video` call.
pub async fn generate_params(stub: &Stub) -> Vec<Value> {
    calls_of(stub, "generate_video")
        .await
        .into_iter()
        .map(|call| call["arguments"]["params"].clone())
        .collect()
}

/// The name of the tool a request calls, if any.
pub fn called_tool(request: &Request) -> Option<String> {
    body_json(request)["params"]["name"].as_str().map(str::to_owned)
}

/// A JSON-RPC answer of `payload` (`result` or `error`) to `request`.
pub fn reply_with(request: &Request, payload: &Value) -> Reply {
    let mut object: Map<String, Value> = payload.as_object().cloned().unwrap_or_default();
    object.insert("jsonrpc".into(), json!("2.0"));
    object.insert("id".into(), body_json(request)["id"].clone());
    Reply::json(&Value::Object(object))
}

/// `Provider.answering`: the MCP stub of `tools` and `call`, except that every call to `name` answers with
/// `payload` merged into the JSON-RPC reply.
pub async fn answering(
    name: &str,
    payload: Value,
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> Stub {
    let mcp = McpServer::new(tools).call(call);
    let name = name.to_owned();
    Stub::replying(move |request| {
        let has_id = body_json(request).get("id").is_some();
        if has_id && called_tool(request).as_deref() == Some(name.as_str()) {
            reply_with(request, &payload)
        } else {
            mcp.respond(request)
        }
    })
    .await
}

/// Stub answers in order, the last one repeated.
#[derive(Debug)]
pub struct Answers(Mutex<Vec<Value>>);

impl Answers {
    pub fn new(values: Vec<Value>) -> Self {
        Self(Mutex::new(values))
    }

    pub fn next(&self) -> Value {
        let mut values = self.0.lock().unwrap();
        if values.len() > 1 {
            values.remove(0)
        } else {
            values.first().cloned().unwrap_or_else(|| json!({}))
        }
    }
}

/// `ProviderUnlimited.explore`.
pub fn explore_tool() -> Value {
    tool(
        "models_explore",
        "Explore models",
        &["action", "type", "model_id", "after", "limit"],
    )
}

/// `ProviderUnlimited.tools`.
pub fn unlimited_tools() -> Vec<Value> {
    let mut tools = provider_tools();
    tools.push(explore_tool());
    tools
}

/// `ProviderUnlimited.page(available:)`: Motion Transfer's model supports `use_unlim`, Object Swap's doesn't;
/// `available` is the `unlim` block's, whose `remaining` is `null` for an unmetered allowance.
pub fn unlimited_page(available: bool) -> Value {
    json!({
        "items": [
            {"id": "hf_mult_motion_control", "supports_unlim": true},
            {"id": "hf_mult_replace_object"},
            {"id": "seedance_2_5"}, {"id": "kling3_0_motion_control"},
        ],
        "has_more": false,
        "unlim": {"available": available, "expires_at": null, "remaining": null},
    })
}
