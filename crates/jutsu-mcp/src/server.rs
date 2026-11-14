//! The Model Context Protocol over stdio, written out by hand: JSON-RPC 2.0, one message per line, and only what an
//! agent needs to drive the app. It speaks both eras of the protocol, chosen per request:
//!
//! - **2026-07-28** is stateless: a request carries its protocol version, the client's capabilities and identity in
//!   `params._meta` under `io.modelcontextprotocol/…`, `server/discover` describes the server, and every result says
//!   `resultType: "complete"` and names the server in its own `_meta`.
//! - **2025-11-25 and earlier** start with `initialize` (then `notifications/initialized`, and `ping` at any time),
//!   which fixes the version and the client for the rest of the process.
//!
//! Every tool is one `AgentCommand` passed through to the app; a failure to reach the app is an `AgentError`, which
//! the model sees as a tool error. A malformed call (not JSON-RPC, an unknown method or tool, params of the wrong
//! shape) is a JSON-RPC error instead.

use crate::{
    app_connection::{App, lost_answer_default},
    descriptions, library_tools,
    tools::Tool,
};
use jutsu_agent::{
    AgentCommand, AgentError, AgentResponse, CreateElementCommand, CreateImageCommand, CreateVideoCommand,
    ErrorCode, FramesCommand, UploadRequest, json, names,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{path::PathBuf, sync::Arc};

/// Newest first. Requests with per-request `_meta` may name any of them; `initialize` negotiates one of the last four,
/// the handshake revisions.
pub const PROTOCOL_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

fn handshake_versions() -> &'static [&'static str] {
    &PROTOCOL_VERSIONS[1..]
}

/// How long a client may keep `server/discover` and `tools/list`: the same for every client and fixed for this
/// binary, which is replaced with the app.
pub const CACHE_MILLISECONDS: i64 = 3_600_000;

mod meta_key {
    pub(super) const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
    pub(super) const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
    pub(super) const CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
    pub(super) const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
}

/// A JSON-RPC error answer, for a call that is malformed rather than one a tool refused.
#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// How one request is served: by the version its `_meta` names, or by the session `initialize` set up.
enum Era {
    PerRequest { version: String, client: Option<String> },
    Session,
}

impl Era {
    /// A request whose `_meta` names a protocol version must name a supported one and carry the client's
    /// capabilities; without one it belongs to the handshake era.
    fn of(params: &Map<String, Value>) -> Result<Self, RpcError> {
        let empty = Map::new();
        let meta = params.get("_meta").and_then(Value::as_object).unwrap_or(&empty);
        let Some(requested) = meta.get(meta_key::PROTOCOL_VERSION) else {
            return Ok(Self::Session);
        };
        let Some(version) = requested.as_str() else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("Invalid params: {} is a string.", meta_key::PROTOCOL_VERSION),
            ));
        };
        if !PROTOCOL_VERSIONS.contains(&version) {
            return Err(RpcError {
                code: UNSUPPORTED_PROTOCOL_VERSION,
                message: "Unsupported protocol version".to_owned(),
                data: Some(
                    json!({ "supported": PROTOCOL_VERSIONS, "requested": names::prefix(version, 64) }),
                ),
            });
        }
        if !meta
            .get(meta_key::CLIENT_CAPABILITIES)
            .is_some_and(Value::is_object)
        {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!(
                    "Invalid params: _meta needs {}, an object.",
                    meta_key::CLIENT_CAPABILITIES
                ),
            ));
        }
        Ok(Self::PerRequest {
            version: version.to_owned(),
            client: client_name(meta.get(meta_key::CLIENT_INFO)),
        })
    }
}

/// The negotiated version and the client's name, for the requests without `_meta` after `initialize`.
#[derive(Clone)]
struct Session {
    version: String,
    client: Option<String>,
}

#[derive(Clone)]
pub struct McpServer {
    version: String,
    app: Arc<dyn App>,
    /// Where the agent's relative paths start.
    working_directory: PathBuf,
    session: Option<Session>,
}

impl McpServer {
    pub fn new(version: &str, app: Arc<dyn App>) -> Self {
        Self::with_working_directory(version, app, crate::inbox::Inbox::current_directory())
    }

    pub fn with_working_directory(version: &str, app: Arc<dyn App>, working_directory: PathBuf) -> Self {
        Self {
            version: version.to_owned(),
            app,
            working_directory,
            session: None,
        }
    }

    /// The reply to one line from the client, a line with its newline; `None` for a notification or a response,
    /// which get none.
    pub async fn reply(&mut self, line: &[u8]) -> Option<Vec<u8>> {
        let Ok(message) = serde_json::from_slice::<Value>(line) else {
            return encode_error(
                &RpcError::new(PARSE_ERROR, "Parse error: not JSON."),
                &Value::Null,
            );
        };
        let Value::Object(object) = message else {
            // Batches were dropped from the protocol in 2025-06-18, and no client sends them over stdio.
            let error = RpcError::new(INVALID_REQUEST, "Invalid request: expected one JSON-RPC object.");
            return encode_error(&error, &Value::Null);
        };
        let Some(method) = object.get("method") else {
            // A response to a request of ours; this server sends none.
            if object.contains_key("result") || object.contains_key("error") {
                return None;
            }
            let id = object.get("id").cloned().unwrap_or(Value::Null);
            return encode_error(
                &RpcError::new(INVALID_REQUEST, "Invalid request: no method."),
                &id,
            );
        };
        // A notification (`notifications/initialized`, `notifications/cancelled`) has no id and gets no answer.
        let id = object.get("id")?;
        if !is_valid_id(id) {
            let error = RpcError::new(INVALID_REQUEST, "Invalid request: id is a string or an integer.");
            return encode_error(&error, &Value::Null);
        }
        let Some(name) = method.as_str() else {
            return encode_error(
                &RpcError::new(INVALID_REQUEST, "Invalid request: method is a string."),
                id,
            );
        };
        let params = match object.get("params") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(value)) => value.clone(),
            Some(_) => {
                return encode_error(
                    &RpcError::new(INVALID_PARAMS, "Invalid params: expected an object."),
                    id,
                );
            }
        };
        if name == "initialize" {
            return self.initialize(&params, id);
        }
        match Era::of(&params) {
            Err(error) => encode_error(&error, id),
            Ok(era) => self.respond(name, &params, &era, id).await,
        }
    }

    async fn respond(
        &self,
        method: &str,
        params: &Map<String, Value>,
        era: &Era,
        id: &Value,
    ) -> Option<Vec<u8>> {
        let (version, client) = match era {
            Era::PerRequest { version, client } => (version.clone(), client.clone()),
            // Clients that skip the handshake get its newest revision.
            Era::Session => (
                self.session.as_ref().map_or_else(
                    || handshake_versions()[0].to_owned(),
                    |session| session.version.clone(),
                ),
                self.session.as_ref().and_then(|session| session.client.clone()),
            ),
        };
        let per_request = matches!(era, Era::PerRequest { .. });
        let result = match method {
            "server/discover" => {
                if !per_request {
                    let message = format!(
                        "Invalid params: server/discover needs _meta with {}.",
                        meta_key::PROTOCOL_VERSION
                    );
                    return encode_error(&RpcError::new(INVALID_PARAMS, message), id);
                }
                Ok(Self::discovery())
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tool_list(&version, per_request)),
            "tools/call" => self.call(params, &version, client).await,
            _ => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("Method not found: {}.", names::prefix(method, 64)),
            )),
        };
        match result {
            Err(error) => encode_error(&error, id),
            Ok(value) => {
                let value = if per_request { self.completed(value) } else { value };
                encode_result(&value, id)
            }
        }
    }

    /// The handshake. A version this server speaks is echoed; any other gets the newest handshake revision, and the
    /// client decides whether it can go on.
    fn initialize(&mut self, params: &Map<String, Value>, id: &Value) -> Option<Vec<u8>> {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let negotiated = requested
            .filter(|version| handshake_versions().contains(version))
            .unwrap_or(handshake_versions()[0]);
        self.session = Some(Session {
            version: negotiated.to_owned(),
            client: client_name(params.get("clientInfo")),
        });
        encode_result(
            &json!({
                "protocolVersion": negotiated,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": self.server_info(),
                "instructions": descriptions::instructions(),
            }),
            id,
        )
    }

    fn server_info(&self) -> Value {
        json!({ "name": "jutsu", "title": "Jutsu Studio", "version": self.version })
    }

    fn discovery() -> Value {
        json!({
            "supportedVersions": PROTOCOL_VERSIONS,
            "capabilities": { "tools": { "listChanged": false } },
            "instructions": descriptions::instructions(),
            "ttlMs": CACHE_MILLISECONDS,
            "cacheScope": "public",
        })
    }

    /// Adds what 2026-07-28 asks of every result: `resultType`, and the server's name in `_meta`.
    fn completed(&self, value: Value) -> Value {
        let Value::Object(mut fields) = value else {
            return value;
        };
        fields.insert("resultType".to_owned(), Value::from("complete"));
        let mut meta = fields
            .get("_meta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        meta.insert(meta_key::SERVER_INFO.to_owned(), self.server_info());
        fields.insert("_meta".to_owned(), Value::Object(meta));
        Value::Object(fields)
    }

    async fn call(
        &self,
        params: &Map<String, Value>,
        version: &str,
        client: Option<String>,
    ) -> Result<Value, RpcError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "Invalid params: tools/call needs name, a string.",
            ));
        };
        let Some(tool) = Tool::parse(name) else {
            return Err(RpcError::new(
                INVALID_PARAMS,
                format!("Unknown tool '{}'.", names::prefix(name, 64)),
            ));
        };
        let empty = Map::new();
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(value)) => value,
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_PARAMS,
                    "Invalid params: arguments is an object.",
                ));
            }
        };
        let mut response = match tool.command(arguments, &self.working_directory) {
            Ok(AgentCommand::CreateVideo(command)) => self.create_video(command, client).await,
            Ok(AgentCommand::CreateImage(command)) => self.create_image(command, client).await,
            Ok(AgentCommand::CreateElement(command)) => self.create_element(command, client).await,
            Ok(AgentCommand::Frames(command)) => self.frames(command, client).await,
            Ok(command) => self.app.send(command, client).await,
            Err(error) => failure(error),
        };
        if let Some(capabilities) = &mut response.capabilities {
            capabilities.note_helper(&self.version);
        }
        Ok(tool_result(&response, is_structured(version)))
    }

    async fn stage(
        &self,
        path: &str,
        request: &UploadRequest,
        client: Option<&String>,
    ) -> Result<String, AgentError> {
        self.app
            .stage(path.to_owned(), request.clone(), client.cloned())
            .await
    }

    /// Hands the request's files to the app, each checked against the request ID before a byte is sent, then sends
    /// the command naming their copies. A request the app already queued with these parameters (`request_known`)
    /// needs no file again: the command goes without them and gets the jobs it queued before.
    async fn create_video(&self, command: CreateVideoCommand, client: Option<String>) -> AgentResponse {
        let request = UploadRequest {
            command: "create_video".to_owned(),
            request_id: command.request_id.clone(),
            fingerprint: command.fingerprint.clone(),
            parameters: command.parameters.clone(),
        };
        let mut staged = command.clone();
        let handed = async {
            if let Some(path) = command
                .video
                .source
                .as_ref()
                .and_then(|source| source.path.as_deref())
            {
                let copy = self.stage(path, &request, client.as_ref()).await?;
                if let Some(source) = &mut staged.video.source {
                    source.path = Some(copy);
                }
            }
            staged.video.images = Vec::new();
            for path in &command.video.images {
                staged
                    .video
                    .images
                    .push(self.stage(path, &request, client.as_ref()).await?);
            }
            if let Some(path) = &command.video.speech {
                staged.video.speech = Some(self.stage(path, &request, client.as_ref()).await?);
            }
            Ok::<(), AgentError>(())
        }
        .await;
        match handed {
            Err(error) if error.code == ErrorCode::REQUEST_KNOWN => {
                if let Some(source) = &mut staged.video.source {
                    source.path = None;
                }
                staged.video.images = Vec::new();
                staged.video.speech = None;
            }
            Err(error) => return failure(error),
            Ok(()) => {}
        }
        self.app.send(AgentCommand::CreateVideo(staged), client).await
    }

    /// `create_image` likewise: its inputs are handed over first, in order, unless the app already took the request.
    async fn create_image(&self, command: CreateImageCommand, client: Option<String>) -> AgentResponse {
        let request = upload_request("create_image", &command.request_id, &command.fingerprint);
        let mut staged = command.clone();
        match self
            .stage_all(&command.image.inputs, &request, client.as_ref())
            .await
        {
            Ok(inputs) => staged.image.inputs = inputs,
            Err(error) if error.code == ErrorCode::REQUEST_KNOWN => staged.image.inputs = Vec::new(),
            Err(error) => return failure(error),
        }
        self.app.send(AgentCommand::CreateImage(staged), client).await
    }

    /// `create_element` likewise: its images are handed over first, in order, unless the app already took the
    /// request.
    async fn create_element(&self, command: CreateElementCommand, client: Option<String>) -> AgentResponse {
        let request = upload_request("create_element", &command.request_id, &command.fingerprint);
        let mut staged = command.clone();
        match self.stage_all(&command.images, &request, client.as_ref()).await {
            Ok(images) => staged.images = images,
            Err(error) if error.code == ErrorCode::REQUEST_KNOWN => staged.images = Vec::new(),
            Err(error) => return failure(error),
        }
        self.app.send(AgentCommand::CreateElement(staged), client).await
    }

    async fn stage_all(
        &self,
        paths: &[String],
        request: &UploadRequest,
        client: Option<&String>,
    ) -> Result<Vec<String>, AgentError> {
        let mut copies = Vec::with_capacity(paths.len());
        for path in paths {
            copies.push(self.stage(path, request, client).await?);
        }
        Ok(copies)
    }

    /// `frames` of the agent's own files: they are handed over first, and the app removes its copies once the frames
    /// are written.
    async fn frames(&self, command: FramesCommand, client: Option<String>) -> AgentResponse {
        if command.files().is_empty() {
            return self.app.send(AgentCommand::Frames(command), client).await;
        }
        let mut staged = command.clone();
        let handed = async {
            let request = library_tools::frames_upload(&command)?;
            if let Some(path) = &command.file {
                staged.file = Some(self.stage(path, &request, client.as_ref()).await?);
            }
            if let Some(path) = &command.compare_file {
                staged.compare_file = Some(self.stage(path, &request, client.as_ref()).await?);
            }
            Ok::<(), AgentError>(())
        }
        .await;
        if let Err(error) = handed {
            return failure(error);
        }
        self.app.send(AgentCommand::Frames(staged), client).await
    }
}

fn upload_request(command: &str, request_id: &str, fingerprint: &str) -> UploadRequest {
    UploadRequest {
        command: command.to_owned(),
        request_id: request_id.to_owned(),
        fingerprint: fingerprint.to_owned(),
        parameters: None,
    }
}

fn failure(error: AgentError) -> AgentResponse {
    AgentResponse {
        error: Some(error),
        ..AgentResponse::default()
    }
}

fn tool_list(version: &str, per_request: bool) -> Value {
    let tools: Vec<Value> = Tool::ALL
        .iter()
        .map(|tool| tool.definition(is_structured(version)))
        .collect();
    let mut list = Map::new();
    list.insert("tools".to_owned(), Value::Array(tools));
    if per_request {
        list.insert("ttlMs".to_owned(), Value::from(CACHE_MILLISECONDS));
        list.insert("cacheScope".to_owned(), Value::from("public"));
    }
    Value::Object(list)
}

/// The app's answer as tool output: its JSON as text for every client, and as structured content for those that
/// read it. A success is the result itself (the capabilities, what `show` opened); a failure is
/// `{"error": {code, message, retryable, next}}` with `isError`, so the model sees what to do next.
pub fn tool_result(response: &AgentResponse, structured: bool) -> Value {
    let answer = match &response.error {
        Some(error) => json!({ "error": encoded(error) }),
        None => success(response),
    };
    let text = json::write(&answer);
    let is_error = answer.get("error").is_some();
    let mut result = Map::new();
    result.insert("content".to_owned(), json!([{ "type": "text", "text": text }]));
    result.insert("isError".to_owned(), Value::Bool(is_error));
    if structured {
        result.insert("structuredContent".to_owned(), answer);
    }
    Value::Object(result)
}

/// The one answer a successful response carries, else `no_answer`.
fn success(response: &AgentResponse) -> Value {
    let answers: [Option<Value>; 15] = [
        response.capabilities.as_ref().map(encoded),
        response.shown.as_ref().map(encoded),
        response.quote.as_ref().map(encoded),
        response.created.as_ref().map(encoded),
        response.jobs.as_ref().map(encoded),
        response.acted.as_ref().map(encoded),
        response.library.as_ref().map(encoded),
        response.details.as_ref().map(encoded),
        response.downloads.as_ref().map(encoded),
        response.elements.as_ref().map(encoded),
        response.marked.as_ref().map(encoded),
        response.delivery.as_ref().map(encoded),
        response.exported.as_ref().map(encoded),
        response.frames.as_ref().map(encoded),
        response.subtitles.as_ref().map(encoded),
    ];
    answers
        .into_iter()
        .flatten()
        .next()
        .unwrap_or_else(|| json!({ "error": encoded(&lost_answer_default()) }))
}

/// A value as the wire writes it: its fields without the ones that are `None`.
fn encoded<T: Serialize>(value: &T) -> Value {
    json::to_value(value).unwrap_or_else(|_| Value::Object(Map::new()))
}

/// Structured tool output and output schemas exist from 2025-06-18 on; the dates sort as strings.
pub fn is_structured(version: &str) -> bool {
    version >= "2025-06-18"
}

/// The client's `title`, else its `name`, cut to 64 characters: shown to the person as where a job came from.
pub fn client_name(info: Option<&Value>) -> Option<String> {
    let info = info?;
    let name = info
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| info.get("name").and_then(Value::as_str))?;
    let name = names::prefix(names::trim_whitespace_and_newlines(name), 64);
    (!name.is_empty()).then(|| name.to_owned())
}

fn is_valid_id(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(number) => {
            number.is_i64()
                || number.is_u64()
                || number.as_f64().is_some_and(|value| {
                    #[allow(clippy::float_cmp, reason = "a whole number rounds to itself exactly")]
                    let whole = value.round() == value;
                    value.is_finite() && whole
                })
        }
        _ => false,
    }
}

fn line(value: &Value) -> Vec<u8> {
    let mut line = json::write(value).into_bytes();
    line.push(b'\n');
    line
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "every reply is optional: a notification gets none"
)]
fn encode_result(result: &Value, id: &Value) -> Option<Vec<u8>> {
    Some(line(&json!({ "jsonrpc": "2.0", "id": id, "result": result })))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "every reply is optional: a notification gets none"
)]
fn encode_error(error: &RpcError, id: &Value) -> Option<Vec<u8>> {
    let mut body = Map::new();
    body.insert("code".to_owned(), Value::from(error.code));
    body.insert("message".to_owned(), Value::from(error.message.as_str()));
    if let Some(data) = &error.data {
        body.insert("data".to_owned(), data.clone());
    }
    Some(line(&json!({ "jsonrpc": "2.0", "id": id, "error": body })))
}
