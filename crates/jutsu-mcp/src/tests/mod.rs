//! The helper's tests, ported from the macOS app's `JutsuMCPTests`, case for case.

mod app_connection;
mod connection;
mod golden;
mod library_tools;
mod on_device_tools;
mod server;
mod subtitles_tools;
mod video_tools;

use crate::{App, McpServer, Tool};
use async_trait::async_trait;
use jutsu_agent::{AgentCommand, AgentError, AgentResponse, UploadRequest, wire};
use serde_json::{Map, Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// What the fake app was asked, what it staged for each file, and what it answers.
pub(crate) struct FakeApp {
    calls: Mutex<Vec<(AgentCommand, Option<String>)>>,
    answer: Mutex<AgentResponse>,
    staged: Mutex<Vec<(String, UploadRequest)>>,
    /// Answered instead of staging, when set.
    refusal: Mutex<Option<AgentError>>,
}

impl FakeApp {
    pub(crate) fn new(answer: AgentResponse) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            answer: Mutex::new(answer),
            staged: Mutex::new(Vec::new()),
            refusal: Mutex::new(None),
        })
    }

    pub(crate) fn calls(&self) -> Vec<(AgentCommand, Option<String>)> {
        self.calls.lock().expect("calls").clone()
    }

    pub(crate) fn commands(&self) -> Vec<AgentCommand> {
        self.calls().into_iter().map(|(command, _)| command).collect()
    }

    pub(crate) fn set_answer(&self, answer: AgentResponse) {
        *self.answer.lock().expect("answer") = answer;
    }

    pub(crate) fn staged(&self) -> Vec<(String, UploadRequest)> {
        self.staged.lock().expect("staged").clone()
    }

    pub(crate) fn staged_paths(&self) -> Vec<String> {
        self.staged().into_iter().map(|(path, _)| path).collect()
    }

    pub(crate) fn refuse(&self, error: AgentError) {
        *self.refusal.lock().expect("refusal") = Some(error);
    }
}

#[async_trait]
impl App for FakeApp {
    async fn send(&self, command: AgentCommand, client: Option<String>) -> AgentResponse {
        self.calls.lock().expect("calls").push((command, client));
        self.answer.lock().expect("answer").clone()
    }

    async fn stage(
        &self,
        path: String,
        request: UploadRequest,
        _client: Option<String>,
    ) -> Result<String, AgentError> {
        if let Some(refusal) = self.refusal.lock().expect("refusal").clone() {
            return Err(refusal);
        }
        let mut staged = self.staged.lock().expect("staged");
        staged.push((path.clone(), request));
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(format!("/tmp/inbox/{}/{name}", staged.len()))
    }
}

pub(crate) fn server(app: &Arc<FakeApp>, version: &str) -> McpServer {
    McpServer::new(version, app.clone())
}

/// The reply to `line`, which must be one whole line of JSON.
pub(crate) async fn reply_to(server: &mut McpServer, line: &[u8]) -> Value {
    let data = server.reply(line).await.expect("a reply");
    assert_eq!(data.last(), Some(&b'\n'));
    assert!(!data[..data.len() - 1].contains(&b'\n'));
    serde_json::from_slice(&data).expect("JSON")
}

/// One request through `server`, and its reply as JSON.
pub(crate) async fn request(server: &mut McpServer, method: &str, params: Option<Value>, id: Value) -> Value {
    let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
    if let Some(params) = params {
        message["params"] = params;
    }
    reply_to(server, &serde_json::to_vec(&message).expect("line")).await
}

/// One `tools/call` through a new server, and its result.
pub(crate) async fn call(app: &Arc<FakeApp>, version: &str, name: &str, arguments: Value) -> Value {
    let mut server = server(app, version);
    let params = json!({ "name": name, "arguments": arguments });
    request(&mut server, "tools/call", Some(params), json!(1)).await["result"].clone()
}

/// The arguments of a `json!` object.
pub(crate) fn object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

/// A tool's command for these arguments, relative paths from `folder`.
pub(crate) fn command_in(tool: Tool, arguments: &Value, folder: &Path) -> Result<AgentCommand, AgentError> {
    tool.command(&object(arguments), folder)
}

/// A tool's command for these arguments, relative paths from the working directory.
pub(crate) fn command(tool: Tool, arguments: &Value) -> Result<AgentCommand, AgentError> {
    command_in(tool, arguments, &crate::Inbox::current_directory())
}

/// A folder of the test's own in the temporary folder, for its files.
pub(crate) struct Files {
    folder: tempfile::TempDir,
}

impl Files {
    pub(crate) fn new() -> Self {
        Self {
            folder: tempfile::Builder::new()
                .prefix("tools")
                .tempdir()
                .expect("folder"),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.folder.path()
    }

    /// A file of `bytes` bytes named `name`, by its path with symlinks followed.
    pub(crate) fn file(&self, name: &str, bytes: usize) -> String {
        let path = self.folder.path().join(name);
        fs::write(&path, vec![1; bytes]).expect("file");
        real(&path).to_string_lossy().into_owned()
    }
}

/// `path` as the helper resolves it: symlinks followed, and on macOS `/private` dropped as Foundation drops it.
pub(crate) fn real(path: &Path) -> PathBuf {
    crate::Inbox::resolve(
        &fs::canonicalize(path).expect("real path").to_string_lossy(),
        Path::new("/"),
    )
}

/// The app's refusal of the same call as its command line: camelCase keys beside `version` and `command`.
pub(crate) fn app_refusal(tool: &str, fields: &Map<String, Value>) -> Option<AgentError> {
    let mut line = fields.clone();
    line.insert("version".to_owned(), json!(1));
    line.insert("command".to_owned(), json!(tool));
    wire::request(&serde_json::to_vec(&line).expect("line")).1.err()
}
