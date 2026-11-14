//! Helpers of the tests agent d ported: the Swift `Provider` suite's `provider(tools:call:)`, `spec(_:)`, `inputs`,
//! `image` and `answering(_:with:tools:call:)`.

use jutsu_core::{
    ClosedRange, JobSpec, LocalImage, Mode, PixelSize, RemoteAsset, RemoteInputs, Resolution, SourceClip,
    VideoModel,
};
use serde_json::Value;
use url::Url;

use super::account::calls;
use super::mcp::{McpServer, Reply};
use super::provider::make_provider;
use super::stub::{Stub, body_json};
use crate::HiggsfieldMcpProvider;

/// A `file://` URL of `path`.
pub fn file_url(path: &str) -> Url {
    Url::parse(&format!("file://{path}")).unwrap()
}

/// An account provider on a stub MCP server listing `tools`, whose `tools/call` answers come from `call`.
pub async fn provider(
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldMcpProvider, Stub) {
    let stub = McpServer::new(tools).call(call).serve().await;
    (make_provider(&stub, false), stub)
}

/// A 10 s 1280×720 source, all of it, at 720p on Genjutsu.
pub fn spec(mode: Mode) -> JobSpec {
    let source = SourceClip::new(
        file_url("/tmp/in.mov"),
        10.0,
        PixelSize::new(1280, 720),
        ClosedRange::new(0.0, 10.0),
    );
    JobSpec::new(mode, VideoModel::GENJUTSU, Some(source), Resolution::P720)
}

/// The uploaded video `video-1` and image `image-1`.
pub fn inputs() -> RemoteInputs {
    RemoteInputs {
        video: Some(RemoteAsset {
            url: file_url("/in.mp4"),
            id: Some("video-1".to_owned()),
        }),
        images: vec![RemoteAsset {
            url: file_url("/a.jpg"),
            id: Some("image-1".to_owned()),
        }],
        audio: None,
    }
}

pub fn image() -> LocalImage {
    LocalImage::new(file_url("/tmp/a.jpg"))
}

/// A stub MCP server that answers every `tools/call` of `name` with `payload` as its whole JSON-RPC message (plus
/// `jsonrpc` and `id`), and everything else as `McpServer` does.
pub async fn answering(
    name: &str,
    payload: Value,
    tools: Vec<Value>,
    call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> Stub {
    let server = McpServer::new(tools).call(call);
    let name = name.to_owned();
    Stub::replying(move |request| {
        let message = body_json(request);
        match message.get("id") {
            Some(id) if message["params"]["name"] == name.as_str() => {
                let mut reply = payload.clone();
                if let Value::Object(object) = &mut reply {
                    object.insert("jsonrpc".to_owned(), Value::String("2.0".to_owned()));
                    object.insert("id".to_owned(), id.clone());
                }
                Reply::json(&reply)
            }
            _ => server.respond(request),
        }
    })
    .await
}

/// The names of the tools the stub was called with.
pub async fn names(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .map(|call| call["name"].clone())
        .collect()
}

/// The `params` of every call the stub got, `Null` where a call had none.
pub async fn params(stub: &Stub) -> Vec<Value> {
    calls(stub)
        .await
        .into_iter()
        .map(|call| call["arguments"].get("params").cloned().unwrap_or(Value::Null))
        .collect()
}

/// JSON lists equal element by element, numbers by value.
pub fn same_list(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| super::catalog::same(a, b))
}
