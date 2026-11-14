//! How the helper reaches the app. It keeps no state: every command opens the socket, sends one request and closes
//! it. When nothing answers, it starts the app beside it, in the background and with no window, and waits a bounded
//! time for it.

use crate::inbox::Inbox;
use async_trait::async_trait;
use jutsu_agent::{
    AgentClient, AgentCommand, AgentEndpoint, AgentError, AgentRequest, AgentResponse, ErrorCode,
    HELPER_LAUNCH_ARGUMENT, HELPER_TIMEOUT, UploadRequest, Uuid, socket,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// What the MCP server needs of the app: the round trip of one command, and the handoff of one of the agent's files.
#[async_trait]
pub trait App: Send + Sync + 'static {
    async fn send(&self, command: AgentCommand, client: Option<String>) -> AgentResponse;

    /// Hands the agent's file at `path` to the app for `request` and returns its path in the app's inbox.
    async fn stage(
        &self,
        path: String,
        _request: UploadRequest,
        _client: Option<String>,
    ) -> Result<String, AgentError> {
        Err(AgentError::new(
            ErrorCode::FILE_REFUSED,
            format!("This helper can't send {path}."),
            false,
            None,
        ))
    }
}

/// Starts the app and brings it forward. The helper binary has the real one; tests pass one that records its calls.
#[async_trait]
pub trait AppLauncher: Send + Sync + 'static {
    /// The app at `app` started in the background, detached from the helper, and given `arguments`.
    async fn start(&self, app: &Path, arguments: &[&str]) -> io::Result<RunningApp>;

    /// Brings the running app forward after `show`.
    async fn activate(&self) {}
}

/// A started app, as far as the helper needs to follow it.
pub struct RunningApp {
    pub launched: Instant,
    pub is_terminated: Box<dyn Fn() -> bool + Send + Sync>,
}

/// The file name of the app's binary, beside the helper.
pub const APP_BINARY: &str = if cfg!(windows) {
    "jutsu-studio.exe"
} else {
    "jutsu-studio"
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// The app's binary beside this helper, to start when nothing answers at its socket.
    Bundled { app: PathBuf, endpoint: AgentEndpoint },
    /// A socket named outright (`JUTSU_AGENT_SOCKET`). Nothing is ever started for it.
    Socket(AgentEndpoint),
    /// No app beside this binary: an app already running still answers at its socket, and nothing is started.
    Unbundled(AgentEndpoint),
    /// The socket has no place: neither a runtime folder nor a home folder is known.
    NoSocket,
}

impl Location {
    /// Where `executable` (symlinks resolved) finds the app: the override, else the binary beside it.
    pub fn locate(executable: &Path, endpoint: Option<AgentEndpoint>) -> Self {
        let Some(endpoint) = endpoint else {
            return Self::NoSocket;
        };
        if endpoint.is_override {
            return Self::Socket(endpoint);
        }
        let executable = fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
        let app = executable.parent().map(|folder| folder.join(APP_BINARY));
        match app {
            Some(app) if app.is_file() => Self::Bundled { app, endpoint },
            _ => Self::Unbundled(endpoint),
        }
    }

    fn endpoint(&self) -> Option<&AgentEndpoint> {
        match self {
            Self::Bundled { endpoint, .. } | Self::Socket(endpoint) | Self::Unbundled(endpoint) => {
                Some(endpoint)
            }
            Self::NoSocket => None,
        }
    }
}

#[derive(Clone)]
pub struct AppConnection {
    pub location: Location,
    launcher: Arc<dyn AppLauncher>,
    /// How long a starting app has to begin answering, counted from its launch.
    pub launch_wait: Duration,
    /// How long one command may wait for its answer: the app answers every one within `ANSWER_DEADLINE` (a `jobs` or
    /// export wait included), so no tool call blocks longer than this.
    pub timeout: Duration,
    /// Where relative file paths start.
    pub inbox: Arc<Inbox>,
}

impl AppConnection {
    pub fn new(location: Location, launcher: Arc<dyn AppLauncher>) -> Self {
        Self {
            location,
            launcher,
            launch_wait: Duration::from_secs(10),
            timeout: HELPER_TIMEOUT,
            inbox: Arc::new(Inbox::new(Inbox::current_directory())),
        }
    }

    /// Sends `command` and returns the app's answer, or an error saying why the app couldn't be reached.
    pub async fn send(&self, command: AgentCommand, client: Option<String>) -> AgentResponse {
        let timeout = self.timeout;
        let is_show = matches!(command, AgentCommand::Show(_));
        let response = self
            .exchange(|endpoint| {
                let mut request = AgentRequest::new(command.clone());
                request.id = Some(Uuid::new_v4().to_string());
                request.client.clone_from(&client);
                async move {
                    AgentClient::connect(&endpoint, timeout)
                        .await?
                        .send(&request)
                        .await
                }
            })
            .await;
        if is_show && matches!(self.location, Location::Bundled { .. }) && response.shown.is_some() {
            self.launcher.activate().await;
        }
        response
    }

    /// Sends the file at `path` into the app's inbox and returns where the app put it. Safe to repeat: each upload
    /// lands in a new folder, so a retry never touches an earlier copy.
    pub async fn stage(
        &self,
        path: &str,
        request: Option<UploadRequest>,
        client: Option<String>,
    ) -> Result<String, AgentError> {
        let (source, command) = self.inbox.upload(path, request)?;
        let timeout = self.timeout;
        let response = self
            .exchange(|endpoint| {
                let (source, command, client) = (source.clone(), command.clone(), client.clone());
                async move {
                    AgentClient::connect(&endpoint, timeout)
                        .await?
                        .upload(&command, &source, None, client.as_deref())
                        .await
                }
            })
            .await;
        if let Some(error) = response.error {
            return Err(error);
        }
        response.uploaded.ok_or_else(|| {
            AgentError::new(
                ErrorCode::FILE_REFUSED,
                format!("Jutsu Studio didn't take {path}."),
                true,
                Some("Update Jutsu Studio so the app and this helper match, then send the request again."),
            )
        })
    }

    /// One round trip through the socket, `body` making it, starting the app beside the helper when nothing answers.
    pub(crate) async fn exchange<F, Fut>(&self, body: F) -> AgentResponse
    where
        F: Fn(AgentEndpoint) -> Fut,
        Fut: Future<Output = Result<AgentResponse, jutsu_agent::Error>>,
    {
        let Some(endpoint) = self.location.endpoint() else {
            return failure(no_socket());
        };
        // The app can't listen there either, so starting it and waiting would only end in a misleading timeout.
        if !endpoint.fits() {
            return failure(AgentError::socket_path_too_long(&endpoint.name));
        }
        match body(endpoint.clone()).await {
            Ok(response) => response,
            Err(jutsu_agent::Error::NotListening(_)) => match &self.location {
                Location::Bundled { app, .. } => self.start_and_send(&body, app, endpoint).await,
                Location::Unbundled(_) => failure(unbundled()),
                _ => failure(not_listening(&endpoint.name)),
            },
            Err(error) => failure(self.lost_answer(&error)),
        }
    }

    /// Nothing listens: the app isn't running or is still starting. A running app that doesn't answer within
    /// `launch_wait` of its launch couldn't open the socket, and Settings › Agents says why.
    async fn start_and_send<F, Fut>(&self, body: &F, app: &Path, endpoint: &AgentEndpoint) -> AgentResponse
    where
        F: Fn(AgentEndpoint) -> Fut,
        Fut: Future<Output = Result<AgentResponse, jutsu_agent::Error>>,
    {
        let running = match self.launcher.start(app, &[HELPER_LAUNCH_ARGUMENT]).await {
            Ok(running) => running,
            Err(error) => return failure(not_started(app, &error.to_string())),
        };
        let deadline = running.launched + self.launch_wait;
        loop {
            match body(endpoint.clone()).await {
                Ok(response) => return response,
                // Not yet.
                Err(jutsu_agent::Error::NotListening(_)) => {}
                Err(error) => return failure(self.lost_answer(&error)),
            }
            if (running.is_terminated)() {
                return failure(not_started(app, "it quit"));
            }
            if Instant::now() >= deadline {
                return failure(not_listening(&endpoint.name));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// A request that went out without an answer coming back, by why: the wait ran out, or the app closed the
    /// connection.
    fn lost_answer(&self, error: &jutsu_agent::Error) -> AgentError {
        match error {
            jutsu_agent::Error::TimedOut => lost_answer(&format!(
                "Jutsu Studio didn't answer within {} seconds: it was busy, or the system held it back in the background.",
                self.timeout.as_secs()
            )),
            jutsu_agent::Error::Closed => {
                lost_answer("Jutsu Studio closed the connection before it answered: it quit or was stopped.")
            }
            other => lost_answer(&format!("Jutsu Studio didn't answer the request: {other}")),
        }
    }
}

#[async_trait]
impl App for AppConnection {
    async fn send(&self, command: AgentCommand, client: Option<String>) -> AgentResponse {
        AppConnection::send(self, command, client).await
    }

    async fn stage(
        &self,
        path: String,
        request: UploadRequest,
        client: Option<String>,
    ) -> Result<String, AgentError> {
        AppConnection::stage(self, &path, Some(request), client).await
    }
}

fn failure(error: AgentError) -> AgentResponse {
    AgentResponse {
        error: Some(error),
        ..AgentResponse::default()
    }
}

pub const NOT_LISTENING: ErrorCode = ErrorCode::named("not_listening");
pub const NOT_STARTED: ErrorCode = ErrorCode::named("app_not_started");
pub const UNBUNDLED: ErrorCode = ErrorCode::named("not_in_app");
pub const NO_SOCKET: ErrorCode = ErrorCode::named("no_socket_path");
pub const LOST_ANSWER: ErrorCode = ErrorCode::named("no_answer");

pub(crate) fn not_listening(socket: &Path) -> AgentError {
    AgentError::new(
        NOT_LISTENING,
        format!("Jutsu Studio doesn't answer at {}.", socket.display()),
        true,
        Some("Ask the person to open Jutsu Studio and check Settings › Agents, then send the request again."),
    )
}

pub(crate) fn not_started(app: &Path, reason: &str) -> AgentError {
    AgentError::new(
        NOT_STARTED,
        format!("Couldn't start {}: {reason}.", app.display()),
        true,
        Some("Ask the person to open Jutsu Studio, then send the request again."),
    )
}

pub(crate) fn unbundled() -> AgentError {
    AgentError::new(
        UNBUNDLED,
        format!("This jutsu binary isn't beside Jutsu Studio ({APP_BINARY}), and no running app answers."),
        false,
        Some(&format!(
            "Run the jutsu that comes with Jutsu Studio, or point {} at a socket.",
            socket::VARIABLE
        )),
    )
}

pub(crate) fn no_socket() -> AgentError {
    AgentError::new(
        NO_SOCKET,
        "Jutsu Studio's agent socket has no place here: no runtime or home folder is set.",
        false,
        Some(&format!("Point {} at a socket.", socket::VARIABLE)),
    )
}

/// The app may have taken the request, so the retry that is safe is the same request: one the app already took
/// answers with what it made.
pub fn lost_answer(message: &str) -> AgentError {
    AgentError::new(
        LOST_ANSWER,
        message,
        true,
        Some(
            "Call capabilities to check that the app is running, then send the request again. Send create_video, \
             create_image, enhance, create_element or subtitles make with the same request_id and parameters: one \
             the app already took returns its job or element and never runs or pays twice.",
        ),
    )
}

/// The request went out but no answer came back.
pub fn lost_answer_default() -> AgentError {
    lost_answer("Jutsu Studio didn't answer the request.")
}
