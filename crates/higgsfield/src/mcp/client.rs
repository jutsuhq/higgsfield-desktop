//! The three MCP calls this app needs (`initialize`, `tools/list`, `tools/call`) as JSON-RPC 2.0 over Streamable
//! HTTP, with a bearer token.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::error::{JsonRpcError, McpError};
use super::http::{is_event_stream, send_within};
use super::json::{self, Object};
use super::oauth::TokenProvider;
use super::sse::server_sent_event_data;
use crate::core::{HiggsfieldError, HttpResponse, sorted_json};

/// A tool from `tools/list`.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
}

impl McpTool {
    /// The tool as the server described it, or `None` without a string `name` or an `inputSchema`.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let description = match object.get("description") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return None,
        };
        Some(Self {
            name: object.get("name")?.as_str()?.to_owned(),
            description,
            input_schema: object.get("inputSchema")?.clone(),
        })
    }

    /// The names of the schema's `properties`.
    pub fn argument_names(&self) -> Vec<String> {
        json::get(&self.input_schema, "properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// The result of `tools/call`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct McpToolResult {
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
}

impl McpToolResult {
    pub(crate) fn from_value(value: &Value) -> Result<Self, McpError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            #[serde(default)]
            content: Option<Vec<Value>>,
            #[serde(default)]
            structured_content: Option<Value>,
            #[serde(default)]
            is_error: Option<bool>,
        }
        let wire = Wire::deserialize(value).map_err(|error| McpError::Decoding(error.to_string()))?;
        Ok(Self {
            content: wire.content.unwrap_or_default(),
            structured_content: wire.structured_content.filter(|value| !value.is_null()),
            is_error: wire.is_error.unwrap_or(false),
        })
    }

    /// `structuredContent`, else the first text block parsed as JSON, else that text as a string.
    #[must_use]
    pub fn value(&self) -> Option<Value> {
        if let Some(structured) = &self.structured_content {
            return Some(structured.clone());
        }
        let text = self
            .content
            .iter()
            .find_map(|block| json::string(block, "text"))?;
        Some(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
    }

    /// The text blocks, joined; servers put error messages here.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| json::string(block, "text"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Published only after both initialization messages succeed. A server may be stateless.
#[derive(Debug, Clone)]
struct Session {
    id: Option<String>,
}

/// Why one exchange in a session failed.
enum Failure {
    /// The server answered `404` to a request with session ID `0`: it ended that session.
    Expired(String),
    Error(McpError),
}

impl<E: Into<McpError>> From<E> for Failure {
    fn from(error: E) -> Self {
        Self::Error(error.into())
    }
}

/// Initializes lazily on the first call, and again after the server ended the session.
pub struct McpClient {
    endpoint: Url,
    http: reqwest::Client,
    tokens: TokenProvider,
    active: Mutex<Option<Session>>,
    /// Held while a handshake runs, so concurrent first calls share one session.
    initializing: tokio::sync::Mutex<()>,
    next_id: AtomicI64,
    /// How long one request may take in all, so a stream that never answers can't hold its job in the queue.
    request_timeout: Duration,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpClient")
            .field("endpoint", &self.endpoint.as_str())
            .finish_non_exhaustive()
    }
}

impl McpClient {
    pub const PROTOCOL_VERSION: &'static str = "2025-06-18";
    /// `tools/list` stops after this many pages, so a server that never ends its cursor chain can't loop.
    pub const MAX_TOOL_PAGES: usize = 50;
    pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

    #[must_use]
    pub fn new(endpoint: Url, tokens: TokenProvider) -> Self {
        Self {
            endpoint,
            http: reqwest::Client::new(),
            tokens,
            active: Mutex::new(None),
            initializing: tokio::sync::Mutex::new(()),
            next_id: AtomicI64::new(1),
            request_timeout: Self::DEFAULT_REQUEST_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn tokens(&self) -> &TokenProvider {
        &self.tokens
    }

    /// A tool that doesn't decode (no `inputSchema`, say) is left out, so it can't hide the others.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        Ok(self
            .list_tool_values()
            .await?
            .iter()
            .filter_map(|value| {
                let tool = McpTool::from_value(value);
                if tool.is_none() {
                    let name = json::string(value, "name").unwrap_or("unnamed");
                    tracing::error!(tool = name, "skipped a malformed tool of tools/list");
                }
                tool
            })
            .collect())
    }

    /// Every page of `tools/list`, each tool exactly as the server described it.
    pub async fn list_tool_values(&self) -> Result<Vec<Value>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..Self::MAX_TOOL_PAGES {
            let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor }));
            let page = self.call("tools/list", params).await?;
            let decoding = || McpError::Decoding("tools/list page".into());
            let listed = json::array(&page, "tools").ok_or_else(decoding)?;
            tools.extend(listed.iter().cloned());
            let next = match json::get(&page, "nextCursor") {
                None | Some(Value::Null) => None,
                Some(Value::String(next)) => Some(next.clone()),
                Some(_) => return Err(decoding()),
            };
            let Some(next) = next.filter(|next| seen.insert(next.clone())) else {
                break;
            };
            cursor = Some(next);
        }
        Ok(tools)
    }

    pub async fn call_tool(&self, name: &str, arguments: Object) -> Result<McpToolResult, McpError> {
        let result = self
            .call(
                "tools/call",
                Some(json!({ "name": name, "arguments": arguments })),
            )
            .await?;
        McpToolResult::from_value(&result)
    }

    /// Opens the session and reads a token, so a call that follows fails before that point only by its own request.
    pub async fn prepare(&self) -> Result<(), McpError> {
        self.initialize().await.map_err(expired_as_404)?;
        self.tokens.access_token().await?;
        Ok(())
    }

    /// Streamable HTTP: a `404` to a request with a session ID means the server ended that session and did not
    /// process the request. The client starts a new session, without the old ID, and sends the request once more.
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        for attempt in 1..=2 {
            match self.call_in_session(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(Failure::Error(error)) => return Err(error),
                Err(Failure::Expired(id)) => {
                    self.end_session(&id);
                    if attempt == 2 {
                        return Err(McpError::server(404));
                    }
                }
            }
        }
        Err(McpError::server(404))
    }

    async fn call_in_session(&self, method: &str, params: Option<Value>) -> Result<Value, Failure> {
        let session = self.initialize().await?;
        Ok(self.request(method, params, Some(&session)).await?.0)
    }

    /// Forgets session `id`, unless a concurrent call already did and started the next one.
    fn end_session(&self, id: &str) {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.as_ref().and_then(|session| session.id.as_deref()) == Some(id) {
            *active = None;
        }
    }

    fn active_session(&self) -> Option<Session> {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn initialize(&self) -> Result<Session, Failure> {
        if let Some(session) = self.active_session() {
            return Ok(session);
        }
        let _handshake = self.initializing.lock().await;
        if let Some(session) = self.active_session() {
            return Ok(session);
        }
        let params = json!({
            "protocolVersion": Self::PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "Jutsu Studio", "version": "0.1.0" },
        });
        let (value, response) = self.request("initialize", Some(params), None).await?;
        if json::string(&value, "protocolVersion") != Some(Self::PROTOCOL_VERSION) {
            return Err(McpError::server(response.status).into());
        }
        let session = Session {
            id: response.header("Mcp-Session-Id").map(str::to_owned),
        };
        self.post(
            &message(None, "notifications/initialized", None),
            Some(&session),
            None,
        )
        .await?;
        *self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session.clone());
        Ok(session)
    }

    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        session: Option<&Session>,
    ) -> Result<(Value, HttpResponse), Failure> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .post(&message(Some(id), method, params), session, Some(id))
            .await?;
        let messages: Vec<String> = if is_event_stream(&response.headers) {
            server_sent_event_data(&String::from_utf8_lossy(&response.body))
        } else {
            vec![String::from_utf8_lossy(&response.body).into_owned()]
        };
        // An event stream may carry server notifications before the response; anything else is skipped.
        let Some(reply) = messages
            .iter()
            .filter_map(|message| serde_json::from_str::<Reply>(message).ok())
            .find(|reply| reply.id == Some(id))
        else {
            return Err(McpError::server(response.status).into());
        };
        if let Some(error) = reply.error {
            return Err(McpError::JsonRpc(error).into());
        }
        Ok((reply.result.unwrap_or(Value::Null), response))
    }

    /// POSTs one message with the bearer token; on `401` refreshes once and retries. An event-stream answer is
    /// read until the reply to `reply_id` arrives, not until the server closes the stream.
    async fn post(
        &self,
        message: &Value,
        session: Option<&Session>,
        reply_id: Option<i64>,
    ) -> Result<HttpResponse, Failure> {
        let body = sorted_json(message);
        let mut token = self.tokens.access_token().await?;
        for attempt in 1..=2 {
            let mut request = self
                .http
                .post(self.endpoint.clone())
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .header("Authorization", format!("Bearer {token}"))
                .body(body.clone());
            if let Some(session) = session {
                if let Some(id) = &session.id {
                    request = request.header("Mcp-Session-Id", id);
                }
                request = request.header("MCP-Protocol-Version", Self::PROTOCOL_VERSION);
            }
            let response = send_within(request, self.request_timeout, |events| {
                reply_id.is_some_and(|id| is_reply(id, events))
            })
            .await?;
            tracing::debug!(
                method = json::string(message, "method"),
                status = response.status,
                "MCP request"
            );
            if response.status == 401 && attempt == 1 {
                token = self.tokens.access_token_replacing(&token).await?;
                continue;
            }
            if response.status == 404
                && let Some(id) = session.and_then(|session| session.id.clone())
            {
                return Err(Failure::Expired(id));
            }
            return match response.status {
                200..=299 => Ok(response),
                401 | 403 => Err(HiggsfieldError::Unauthorized.into()),
                status => Err(HiggsfieldError::Server { status }.into()),
            };
        }
        Err(HiggsfieldError::Unauthorized.into())
    }
}

/// An expired session outside `call`, which retries it, is a `404`.
fn expired_as_404(failure: Failure) -> McpError {
    match failure {
        Failure::Expired(_) => McpError::server(404),
        Failure::Error(error) => error,
    }
}

fn message(id: Option<i64>, method: &str, params: Option<Value>) -> Value {
    let mut message = Object::new();
    message.insert("jsonrpc".into(), "2.0".into());
    if let Some(id) = id {
        message.insert("id".into(), id.into());
    }
    message.insert("method".into(), method.into());
    if let Some(params) = params {
        message.insert("params".into(), params);
    }
    Value::Object(message)
}

#[derive(Deserialize)]
struct Reply {
    id: Option<i64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

/// Whether the event stream read so far ends with the reply to `id`. The reader asks after every event, so only
/// the newest one is decoded: each event once, however long the stream.
pub(crate) fn is_reply(id: i64, events: &[String]) -> bool {
    events
        .last()
        .and_then(|event| serde_json::from_str::<Reply>(event).ok())
        .and_then(|reply| reply.id)
        == Some(id)
}
