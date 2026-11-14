//! A stub Streamable HTTP MCP server and the fixtures of the MCP tests.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use higgsfield::mcp::oauth::{AuthorizationServerMetadata, OAuthClient, TokenProvider, TokenSet};
use serde_json::{Value, json};
use time::OffsetDateTime;
use url::Url;
use wiremock::{Request, ResponseTemplate};

use super::stub::{Stub, body_json, header};

/// A stub answer the tests can still change before it is sent, as the Swift `StubResponse`.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Holds the answer back this long: a reply that never comes in time.
    pub delay: Option<Duration>,
}

impl Reply {
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: String::new(),
            delay: None,
        }
    }

    pub fn not_found() -> Self {
        Self::status(404)
    }

    pub fn json(value: &Value) -> Self {
        Self::json_text(&serde_json::to_string(value).unwrap())
    }

    pub fn json_text(text: &str) -> Self {
        Self::status(200)
            .header("Content-Type", "application/json")
            .body(text)
    }

    pub fn event_stream(text: &str) -> Self {
        Self::status(200)
            .header("Content-Type", "text/event-stream")
            .body(text)
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.set_header(name, value);
        self
    }

    #[must_use]
    pub fn body(mut self, body: &str) -> Self {
        body.clone_into(&mut self.body);
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        self.remove_header(name);
        self.headers.push((name.to_owned(), value.to_owned()));
    }

    pub fn remove_header(&mut self, name: &str) {
        self.headers.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    }

    pub fn template(self) -> ResponseTemplate {
        let content_type = self
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("Content-Type"))
            .map_or_else(|| "text/plain".to_owned(), |(_, value)| value.clone());
        let mut template =
            ResponseTemplate::new(self.status).set_body_raw(self.body.into_bytes(), &content_type);
        for (name, value) in self
            .headers
            .iter()
            .filter(|(key, _)| !key.eq_ignore_ascii_case("Content-Type"))
        {
            template = template.insert_header(name.as_str(), value.as_str());
        }
        if let Some(delay) = self.delay {
            template = template.set_delay(delay);
        }
        template
    }
}

impl Stub {
    /// A TLS stub answering every request with `handler`'s `Reply`.
    pub async fn replying(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        Self::serve(move |request| handler(request).template()).await
    }
}

/// The `WWW-Authenticate` of the stub's `401`, shaped like the real server's.
pub const DEFAULT_BEARER_CHALLENGE: &str =
    r#"Bearer resource_metadata="https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp""#;

type Tools = Arc<dyn Fn() -> Vec<Value> + Send + Sync>;
type Call = Arc<dyn Fn(&str, &Value) -> Value + Send + Sync>;

/// A Streamable HTTP MCP server: JSON for `initialize`, SSE for everything else, a session ID, and `401` for any
/// bearer token in `rejected`. `call` answers `tools/call` with structured content.
#[derive(Clone)]
pub struct McpServer {
    tools: Tools,
    page_size: usize,
    rejected: HashSet<String>,
    www_authenticate: String,
    call: Call,
}

impl McpServer {
    pub fn new(tools: Vec<Value>) -> Self {
        Self::changing(move || tools.clone())
    }

    /// A tool list that may change: `tools` is read again for every `tools/list` page.
    pub fn changing(tools: impl Fn() -> Vec<Value> + Send + Sync + 'static) -> Self {
        Self {
            tools: Arc::new(tools),
            page_size: 100,
            rejected: HashSet::new(),
            www_authenticate: DEFAULT_BEARER_CHALLENGE.to_owned(),
            call: Arc::new(|_, _| json!({})),
        }
    }

    #[must_use]
    pub fn page_size(mut self, size: usize) -> Self {
        self.page_size = size;
        self
    }

    #[must_use]
    pub fn rejected(mut self, tokens: &[&str]) -> Self {
        self.rejected = tokens.iter().map(|token| (*token).to_owned()).collect();
        self
    }

    #[must_use]
    pub fn call(mut self, call: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> Self {
        self.call = Arc::new(call);
        self
    }

    pub async fn serve(self) -> Stub {
        Stub::replying(move |request| self.respond(request)).await
    }

    pub fn respond(&self, request: &Request) -> Reply {
        let bearer = header(request, "Authorization").unwrap_or("");
        if self
            .rejected
            .contains(bearer.strip_prefix("Bearer ").unwrap_or(bearer))
        {
            return Reply::status(401).header("WWW-Authenticate", &self.www_authenticate);
        }
        let message = body_json(request);
        let Some(method) = message["method"].as_str() else {
            return Reply::not_found();
        };
        let Some(id) = message.get("id").cloned() else {
            return Reply::status(202);
        };
        let result = match method {
            "initialize" => {
                let reply = json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "stub"},
                    },
                });
                return Reply::json(&reply).header("Mcp-Session-Id", "session-1");
            }
            "tools/list" => {
                let tools = (self.tools)();
                let start = message["params"]["cursor"]
                    .as_str()
                    .and_then(|cursor| cursor.parse().ok())
                    .unwrap_or(0);
                let end = (start + self.page_size).min(tools.len());
                let mut page = json!({ "tools": tools[start.min(end)..end] });
                if end < tools.len() {
                    page["nextCursor"] = Value::String(end.to_string());
                }
                page
            }
            "tools/call" => {
                let name = message["params"]["name"].as_str().unwrap_or("");
                let arguments = message["params"]
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let structured = (self.call)(name, &arguments);
                json!({ "content": [{"type": "text", "text": "done"}], "structuredContent": structured })
            }
            _ => {
                return Reply::json(
                    &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}),
                );
            }
        };
        let reply = serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap();
        let progress = r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{"progress":1}}"#;
        Reply::event_stream(&format!(
            "event: message\ndata: {progress}\n\nevent: message\ndata: {reply}\n\n"
        ))
    }
}

pub fn tool(name: &str, description: &str, properties: &[&str]) -> Value {
    let properties: serde_json::Map<String, Value> = properties
        .iter()
        .map(|key| ((*key).to_owned(), json!({"type": "string"})))
        .collect();
    json!({
        "name": name, "description": description,
        "inputSchema": {"type": "object", "properties": properties},
    })
}

/// A tool whose one input is a `params` object with `keys`, as the operations on a result take theirs.
pub fn params_tool(name: &str, keys: &[&str]) -> Value {
    let properties: serde_json::Map<String, Value> = keys
        .iter()
        .map(|key| ((*key).to_owned(), json!({"type": "string"})))
        .collect();
    json!({
        "name": name, "description": "Works on an existing video",
        "inputSchema": {
            "type": "object", "required": ["params"],
            "properties": {"params": {"type": "object", "properties": properties}},
        },
    })
}

/// The JSON-RPC method of a request, if any.
pub fn method(request: &Request) -> Option<String> {
    body_json(request)["method"].as_str().map(str::to_owned)
}

/// A value shared between a test and the closures it hands out.
#[derive(Debug, Default)]
pub struct Locked<T>(Mutex<T>);

impl<T: Clone> Locked<T> {
    pub fn new(value: T) -> Arc<Self> {
        Arc::new(Self(Mutex::new(value)))
    }

    pub fn value(&self) -> T {
        self.0.lock().unwrap().clone()
    }

    pub fn update<R>(&self, body: impl FnOnce(&mut T) -> R) -> R {
        body(&mut self.0.lock().unwrap())
    }
}

/// Documents as fetched from the live servers on 2026-10-13, verbatim.
pub mod fixture {
    use super::*;

    pub const MCP_URL: &str = "https://mcp.higgsfield.ai/mcp";
    pub const RESOURCE_METADATA_URL: &str =
        "https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp";
    pub const WWW_AUTHENTICATE: &str = concat!(
        r#"Bearer resource_metadata="https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp", "#,
        r#"scope="openid email offline_access""#
    );
    pub const CLERK: &str = "https://clerk.higgsfield.ai";
    pub const DEVICE_AUTH: &str = "https://fnf-device-auth.higgsfield.ai";
    /// The `iss` query item clerk puts on its redirect (RFC 9207), percent-encoded.
    pub const CLERK_ISS: &str = "iss=https%3A%2F%2Fclerk.higgsfield.ai";
    /// Where `docs/oauth-client.json` would be served.
    pub const METADATA_DOCUMENT_URL: &str = "https://jutsuhq.github.io/jutsu-studio/oauth-client.json";

    pub const RESOURCE_METADATA: &str = include_str!("../../fixtures/oauth/resource-metadata.json");

    pub const CLERK_METADATA: &str = include_str!("../../fixtures/oauth/clerk-metadata.json");

    /// Served by the MCP host itself, but not listed in `authorization_servers`.
    pub const MCP_HOST_METADATA: &str = include_str!("../../fixtures/oauth/mcp-host-metadata.json");

    pub const NOT_FOUND_BODY: &str = r#"{"detail":"Not Found"}"#;

    pub const TOKEN_RESPONSE: &str =
        r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#;

    pub fn mcp_url() -> Url {
        Url::parse(MCP_URL).unwrap()
    }

    /// The real clerk document minus Client ID Metadata Document support, to force dynamic registration.
    pub fn clerk_without_cimd() -> String {
        CLERK_METADATA.replace(
            r#""client_id_metadata_document_supported":true"#,
            r#""client_id_metadata_document_supported":false"#,
        )
    }

    pub fn server(json: &str) -> AuthorizationServerMetadata {
        serde_json::from_str(json).unwrap()
    }

    /// An `OAuthClient` with client ID `client-1` for the MCP resource, on `http`, at `now`.
    pub fn client(
        server: AuthorizationServerMetadata,
        http: reqwest::Client,
        now: OffsetDateTime,
    ) -> OAuthClient {
        OAuthClient::new(server, "client-1", mcp_url())
            .with_http_client(http)
            .with_clock(Arc::new(move || now))
    }

    pub fn tokens(access: &str, refresh: Option<&str>, expires_at: OffsetDateTime) -> TokenSet {
        TokenSet::new(
            access,
            refresh.map(str::to_owned),
            expires_at,
            OAuthClient::DEFAULT_SCOPE,
            "client-1",
        )
    }

    /// Swift's `.distantFuture`.
    pub fn distant_future() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(64_092_211_200).unwrap()
    }

    /// Swift's `.distantPast`.
    pub fn distant_past() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(-62_135_596_800).unwrap()
    }
}

/// Tokens that never expire; a refresh counts in `refreshes` and returns access token `fresh`.
pub fn token_provider_counting(access: &str, refreshes: Arc<Locked<usize>>) -> TokenProvider {
    TokenProvider::new(
        fixture::tokens(access, Some("refresh-1"), fixture::distant_future()),
        move |old: TokenSet| {
            refreshes.update(|count| *count += 1);
            async move {
                Ok(fixture::tokens(
                    "fresh",
                    old.refresh_token.as_deref(),
                    fixture::distant_future(),
                ))
            }
        },
        |_| async {},
    )
}

pub fn token_provider(access: &str) -> TokenProvider {
    token_provider_counting(access, Locked::new(0))
}

/// `structuredContent` of `show_generations` with `{"size": 5}`, recorded on 2026-10-14. IDs, paths and long
/// prompts are placeholders, and the page is cut to its first item.
pub const RECORDED_GENERATIONS: &str = r#"
{
  "items": [
    {
      "id": "<job id>",
      "type": "video",
      "status": "completed",
      "model": "hf_mult_replace_object",
      "createdAt": 1791032606.742653,
      "liked": false,
      "params": {
        "prompt": "<1712 characters>",
        "enhanced_prompt": "<3251 characters>",
        "resolution": "720p",
        "aspect_ratio": "auto",
        "width": 1080,
        "height": 1920,
        "duration": 8,
        "batch_size": 1,
        "generate_audio": false,
        "medias": [
          {"role": "video", "data": {"id": "<media id>", "type": "video_input", "url": "https://d2ol7oe51mr4n9.cloudfront.net/<path>"}},
          {"role": "image", "data": {"id": "<media id>", "type": "media_input", "url": "https://d2ol7oe51mr4n9.cloudfront.net/<path>"}},
          {"role": "image", "data": {"id": "<media id>", "type": "media_input", "url": "https://d2ol7oe51mr4n9.cloudfront.net/<path>"}},
          {"role": "image", "data": {"id": "<media id>", "type": "media_input", "url": "https://d2ol7oe51mr4n9.cloudfront.net/<path>"}}
        ]
      },
      "results": {"rawUrl": "https://d8j0ntlcm91z4.cloudfront.net/<user id>/hf_<yyyyMMdd>_<HHmmss>_<job id>.mp4"}
    }
  ],
  "next_cursor": "<cursor; value not recorded>"
}
"#;

pub fn recorded_generations() -> Value {
    serde_json::from_str(RECORDED_GENERATIONS).unwrap()
}
