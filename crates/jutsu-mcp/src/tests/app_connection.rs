//! How the helper finds the app, starts it and reaches it. The macOS app's ad hoc build case has no counterpart: the
//! socket's place no longer depends on how the app was signed.

use crate::{
    AppConnection, AppLauncher, Location, RunningApp,
    app_connection::{APP_BINARY, LOST_ANSWER, NOT_LISTENING, NOT_STARTED, UNBUNDLED},
};
use async_trait::async_trait;
use jutsu_agent::{
    AgentCommand, AgentEndpoint, AgentError, AgentHandler, AgentPage, AgentRequest, AgentResponse,
    AgentServer, AgentShown, CapabilitiesCommand, ErrorCode, HELPER_LAUNCH_ARGUMENT, ShowCommand, socket,
};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Records what the helper asked to start; it never starts anything.
struct FakeLauncher {
    starts: Mutex<Vec<PathBuf>>,
    arguments: Mutex<Vec<Vec<String>>>,
    activations: Mutex<usize>,
    /// What a start does: an app that quits at once, so the wait for it ends without sleeping.
    quits: bool,
}

impl FakeLauncher {
    fn new(quits: bool) -> Arc<Self> {
        Arc::new(Self {
            starts: Mutex::new(Vec::new()),
            arguments: Mutex::new(Vec::new()),
            activations: Mutex::new(0),
            quits,
        })
    }

    fn starts(&self) -> Vec<PathBuf> {
        self.starts.lock().expect("starts").clone()
    }
}

#[async_trait]
impl AppLauncher for FakeLauncher {
    async fn start(&self, app: &Path, arguments: &[&str]) -> io::Result<RunningApp> {
        self.starts.lock().expect("starts").push(app.to_path_buf());
        self.arguments
            .lock()
            .expect("arguments")
            .push(arguments.iter().map(ToString::to_string).collect());
        let quits = self.quits;
        Ok(RunningApp {
            launched: Instant::now(),
            is_terminated: Box::new(move || quits),
        })
    }

    async fn activate(&self) {
        *self.activations.lock().expect("activations") += 1;
    }
}

/// The app's side: answers `show` with what it opened, or never answers when `hangs`.
struct Handler {
    hangs: bool,
}

#[async_trait]
impl AgentHandler for Handler {
    fn agents_allowed(&self) -> bool {
        true
    }

    async fn handle(&self, request: AgentRequest) -> Result<AgentResponse, AgentError> {
        if self.hangs {
            return std::future::pending().await;
        }
        let AgentCommand::Show(ShowCommand { page: Some(page), .. }) = request.command else {
            return Ok(AgentResponse::default());
        };
        assert_eq!(request.client.as_deref(), Some("Agent CLI"));
        Ok(AgentResponse {
            shown: Some(AgentShown {
                page,
                job: None,
                item: None,
                sheet: None,
            }),
            ..AgentResponse::default()
        })
    }
}

/// A folder of the test's own, so its socket and inbox are too.
fn folder() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("jm").tempdir().expect("folder")
}

/// A socket nobody listens on.
fn nowhere(folder: &tempfile::TempDir) -> AgentEndpoint {
    AgentEndpoint::at(folder.path().join(socket::FILE_NAME))
}

/// The app's own socket, not an override.
fn default_socket(folder: &tempfile::TempDir) -> AgentEndpoint {
    AgentEndpoint {
        is_override: false,
        ..nowhere(folder)
    }
}

fn bundled(endpoint: AgentEndpoint) -> Location {
    Location::Bundled {
        app: PathBuf::from("/opt/jutsu-studio").join(APP_BINARY),
        endpoint,
    }
}

/// A running app at `endpoint`, inside the test's runtime.
fn running(endpoint: AgentEndpoint, hangs: bool) -> AgentServer {
    let server =
        AgentServer::with_answer_deadline(endpoint, Arc::new(Handler { hangs }), Duration::from_secs(30));
    server.start().expect("listening");
    server
}

fn capabilities() -> AgentCommand {
    AgentCommand::Capabilities(CapabilitiesCommand::default())
}

fn show(page: AgentPage) -> AgentCommand {
    AgentCommand::Show(ShowCommand::page(page))
}

#[test]
fn the_override_wins_and_names_the_socket_outright() {
    let variables = |name: &str| match name {
        socket::VARIABLE => Some(OsString::from("/tmp/dev.sock")),
        "HOME" => Some(OsString::from("/home/someone")),
        _ => None,
    };
    let endpoint = AgentEndpoint::from_environment(variables);
    let location = Location::locate(Path::new("/usr/local/bin/jutsu"), endpoint);
    let Location::Socket(endpoint) = location else {
        panic!("not the override: {location:?}")
    };
    assert!(endpoint.is_override);
    if cfg!(unix) {
        assert_eq!(endpoint.name, Path::new("/tmp/dev.sock"));
    }
}

#[test]
fn a_bundled_helper_finds_its_app_and_the_apps_socket() {
    let folder = folder();
    let helper = folder
        .path()
        .join(if cfg!(windows) { "jutsu.exe" } else { "jutsu" });
    fs::write(&helper, b"helper").expect("helper");
    fs::write(folder.path().join(APP_BINARY), b"app").expect("app");
    let endpoint = default_socket(&folder);
    let location = Location::locate(&helper, Some(endpoint.clone()));
    let Location::Bundled { app, endpoint: found } = location else {
        panic!("not bundled: {location:?}")
    };
    assert_eq!(
        app,
        fs::canonicalize(folder.path().join(APP_BINARY)).expect("app")
    );
    assert_eq!(found, endpoint);
}

#[tokio::test]
async fn a_helper_outside_an_app_says_so() {
    let folder = folder();
    let helper = folder.path().join("jutsu");
    fs::write(&helper, b"helper").expect("helper");
    let location = Location::locate(&helper, Some(default_socket(&folder)));
    assert!(matches!(location, Location::Unbundled(_)), "{location:?}");
    let launcher = FakeLauncher::new(true);
    let answer = AppConnection::new(location, launcher.clone())
        .send(show(AgentPage::Queue), None)
        .await;
    assert_eq!(answer.error.map(|error| error.code), Some(UNBUNDLED));
    assert_eq!(launcher.starts().len(), 0);
}

#[tokio::test]
async fn the_override_never_starts_the_app() {
    let folder = folder();
    let launcher = FakeLauncher::new(true);
    let answer = AppConnection::new(Location::Socket(nowhere(&folder)), launcher.clone())
        .send(capabilities(), None)
        .await;
    let error = answer.error.expect("an error");
    assert_eq!(error.code, NOT_LISTENING);
    assert!(error.retryable);
    assert_eq!(launcher.starts().len(), 0);
}

#[tokio::test]
async fn a_bundled_app_that_isnt_running_is_started_once() {
    let folder = folder();
    let launcher = FakeLauncher::new(true);
    let location = bundled(default_socket(&folder));
    let answer = AppConnection::new(location, launcher.clone())
        .send(capabilities(), None)
        .await;
    assert_eq!(answer.error.map(|error| error.code), Some(NOT_STARTED));
    assert_eq!(
        launcher.starts(),
        [PathBuf::from("/opt/jutsu-studio").join(APP_BINARY)]
    );
    // With no window and without the focus: the app reads this as a helper launch.
    assert_eq!(
        *launcher.arguments.lock().expect("arguments"),
        [vec![HELPER_LAUNCH_ARGUMENT.to_owned()]]
    );
}

#[tokio::test]
async fn a_socket_path_too_long_is_refused_without_starting_anything() {
    let launcher = FakeLauncher::new(true);
    let endpoint = AgentEndpoint::at(std::env::temp_dir().join("x".repeat(300)));
    let answer = AppConnection::new(bundled(endpoint), launcher.clone())
        .send(show(AgentPage::Create), None)
        .await;
    assert_eq!(
        answer.error.map(|error| error.code),
        Some(ErrorCode::SOCKET_PATH_TOO_LONG)
    );
    assert_eq!(launcher.starts().len(), 0);
}

#[tokio::test]
async fn a_running_app_answers_and_show_brings_it_forward() {
    let folder = folder();
    let server = running(default_socket(&folder), false);
    let launcher = FakeLauncher::new(true);
    let connection = AppConnection::new(bundled(default_socket(&folder)), launcher.clone());
    let answer = connection
        .send(show(AgentPage::Library), Some("Agent CLI".to_owned()))
        .await;
    server.stop();
    assert_eq!(
        answer.shown,
        Some(AgentShown {
            page: AgentPage::Library,
            job: None,
            item: None,
            sheet: None
        })
    );
    assert_eq!(launcher.starts().len(), 0);
    assert_eq!(*launcher.activations.lock().expect("activations"), 1);
}

/// An answer that doesn't come in time says so, and that the same `request_id` is the safe retry.
#[tokio::test]
async fn an_answer_that_never_came_says_why_and_how_to_retry_safely() {
    let folder = folder();
    let server = running(nowhere(&folder), true);
    let mut connection = AppConnection::new(Location::Socket(nowhere(&folder)), FakeLauncher::new(true));
    connection.timeout = Duration::from_secs(1);
    let answer = connection.send(show(AgentPage::Queue), None).await;
    server.stop();
    let error = answer.error.expect("an error");
    assert_eq!(error.code, LOST_ANSWER);
    assert!(error.retryable);
    assert!(error.message.contains("within 1 seconds"), "{}", error.message);
    assert!(error.next.is_some_and(|next| next.contains("same request_id")));
}

#[tokio::test]
async fn a_staged_file_lands_in_the_inbox() {
    let folder = folder();
    let server = running(nowhere(&folder), false);
    let source = folder.path().join("source.png");
    fs::write(&source, b"png").expect("source");
    let connection = AppConnection::new(Location::Socket(nowhere(&folder)), FakeLauncher::new(true));
    let staged = connection
        .stage(&source.to_string_lossy(), None, None)
        .await
        .expect("staged");
    let missing = connection
        .stage(&folder.path().join("no-such-file").to_string_lossy(), None, None)
        .await;
    server.stop();
    assert!(Path::new(&staged).starts_with(&server.inbox.folder), "{staged}");
    assert_eq!(fs::read(&staged).expect("copy"), b"png");
    assert!(missing.is_err());
}
