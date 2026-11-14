//! The listener on real local sockets in the system's temporary folder.

use crate::{
    AgentClient, AgentCommand, AgentEndpoint, AgentError, AgentHandler, AgentInbox, AgentPage, AgentRequest,
    AgentResponse, AgentServer, AgentShown, CapabilitiesCommand, Error, ErrorCode, ShowCommand,
    UploadCommand, inbox, socket,
};
use async_trait::async_trait;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::Notify;

/// A handler a test sets up: Allow agents, a refusal for uploads, the free space, and what it answers.
#[derive(Default)]
struct Handler {
    allowed: AtomicBool,
    handled: AtomicUsize,
    refuse_uploads: Option<AgentError>,
    free_space: Option<i64>,
    activity: Mutex<Vec<bool>>,
    held_while_handling: Mutex<Vec<bool>>,
    /// Holds each command until notified.
    release: Option<Arc<Notify>>,
    finished: AtomicBool,
}

impl Handler {
    fn allowing(allowed: bool) -> Self {
        Self {
            allowed: AtomicBool::new(allowed),
            ..Self::default()
        }
    }
}

#[async_trait]
impl AgentHandler for Handler {
    fn agents_allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    fn check_upload(&self, _upload: &UploadCommand, _client: Option<&str>) -> Result<(), AgentError> {
        self.refuse_uploads.clone().map_or(Ok(()), Err)
    }

    async fn handle(&self, _request: AgentRequest) -> Result<AgentResponse, AgentError> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        let held = self
            .activity
            .lock()
            .expect("activity")
            .last()
            .copied()
            .unwrap_or(false);
        self.held_while_handling.lock().expect("held").push(held);
        if let Some(release) = &self.release {
            release.notified().await;
        }
        self.finished.store(true, Ordering::SeqCst);
        Ok(AgentResponse {
            shown: Some(AgentShown {
                page: AgentPage::Queue,
                job: None,
                item: None,
                sheet: None,
            }),
            ..AgentResponse::default()
        })
    }

    fn free_space(&self, _path: &Path) -> Option<i64> {
        self.free_space
    }

    fn hold_activity(&self, serving: bool) {
        self.activity.lock().expect("activity").push(serving);
    }
}

/// A socket of its own in the temporary folder; on Windows a pipe of its own, its inbox in that folder.
fn temporary_socket() -> (TempDir, AgentEndpoint) {
    let folder = tempfile::Builder::new()
        .prefix("ja")
        .tempdir()
        .expect("temporary folder");
    let mut endpoint = AgentEndpoint::at(folder.path().join(socket::FILE_NAME));
    if cfg!(windows) {
        let unique = folder.path().file_name().expect("folder name").to_string_lossy();
        endpoint.name = PathBuf::from(format!("{}jutsu-test-{unique}", socket::PIPE_NAMESPACE));
        endpoint.folder = folder.path().to_path_buf();
    }
    (folder, endpoint)
}

fn started(handler: Handler) -> (TempDir, AgentServer, Arc<Handler>) {
    let (folder, endpoint) = temporary_socket();
    let handler = Arc::new(handler);
    let server = AgentServer::new(endpoint, handler.clone());
    server.start().expect("listening");
    (folder, server, handler)
}

async fn client(server: &AgentServer) -> AgentClient {
    AgentClient::connect(&server.endpoint, Duration::from_secs(5))
        .await
        .expect("connected")
}

fn request(command: AgentCommand, id: Option<&str>) -> AgentRequest {
    let mut request = AgentRequest::new(command);
    request.id = id.map(str::to_owned);
    request
}

fn show_queue() -> AgentRequest {
    request(AgentCommand::Show(ShowCommand::page(AgentPage::Queue)), None)
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

#[tokio::test]
async fn every_command_is_refused_while_agents_are_off_even_on_an_open_connection() {
    let (_folder, server, handler) = started(Handler::allowing(false));
    let capabilities = request(
        AgentCommand::Capabilities(CapabilitiesCommand::default()),
        Some("1"),
    );
    let mut client = client(&server).await;
    let off = client.send(&capabilities).await.expect("answer");
    handler.allowed.store(true, Ordering::SeqCst);
    let on = client.send(&capabilities).await.expect("answer");
    handler.allowed.store(false, Ordering::SeqCst);
    let off_again = client.send(&capabilities).await.expect("answer");
    let refusal = off.error.expect("refused");
    assert_eq!(refusal.code, ErrorCode::AGENTS_OFF);
    assert!(refusal.retryable);
    assert_eq!(off.id.as_deref(), Some("1"));
    assert!(on.shown.is_some() && on.error.is_none());
    assert_eq!(off_again.error, Some(AgentError::agents_off()));
    assert_eq!(handler.handled.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_malformed_line_gets_an_error_and_the_connection_stays_usable() {
    let (_folder, server, _handler) = started(Handler::allowing(true));
    let mut client = client(&server).await;
    let bad = client.send_line(b"{nope\n").await.expect("answer");
    let good = client
        .send(&request(
            AgentCommand::Show(ShowCommand::page(AgentPage::Library)),
            None,
        ))
        .await;
    assert_eq!(bad.error.map(|error| error.code), Some(ErrorCode::BAD_REQUEST));
    assert!(good.expect("answer").shown.is_some());
}

/// A command whose work outlasts the answer deadline answers `still_working`, and the work goes on to finish.
#[tokio::test]
async fn a_slow_command_answers_still_working_and_keeps_running() {
    let release = Arc::new(Notify::new());
    let handler = Arc::new(Handler {
        release: Some(release.clone()),
        ..Handler::allowing(true)
    });
    let (_folder, endpoint) = temporary_socket();
    let server = AgentServer::with_answer_deadline(endpoint, handler.clone(), Duration::from_millis(20));
    server.start().expect("listening");
    let answer = client(&server)
        .await
        .send(&request(show_queue().command, Some("7")))
        .await
        .expect("answer");
    let refusal = answer.error.expect("still working");
    assert_eq!(refusal.code, ErrorCode::STILL_WORKING);
    assert!(refusal.retryable);
    assert_eq!(answer.id.as_deref(), Some("7"));
    assert!(!handler.finished.load(Ordering::SeqCst));
    release.notify_one();
    eventually(|| handler.finished.load(Ordering::SeqCst)).await;
}

/// Held from the first connection until the last one closes, so the app can't be throttled while it serves.
#[tokio::test]
async fn the_app_is_kept_awake_while_it_serves_an_agent() {
    let (_folder, server, handler) = started(Handler::allowing(true));
    assert_eq!(handler.activity.lock().expect("activity").len(), 0);
    for _ in 0..2 {
        let mut client = client(&server).await;
        let first = client.send(&show_queue()).await.expect("answer");
        let second = client.send(&show_queue()).await.expect("answer");
        assert!(first.shown.is_some() && second.shown.is_some());
        drop(client);
        eventually(|| handler.activity.lock().expect("activity").last() == Some(&false)).await;
    }
    assert_eq!(
        *handler.held_while_handling.lock().expect("held"),
        [true, true, true, true]
    );
    assert_eq!(
        *handler.activity.lock().expect("activity"),
        [true, false, true, false]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_socket_is_the_users_alone() {
    use std::os::unix::fs::PermissionsExt;
    let (_folder, server, _handler) = started(Handler::allowing(true));
    let mode = fs::metadata(&server.endpoint.name)
        .expect("socket")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

/// The app's own folder is made private; an override's folder is left as it is.
#[cfg(unix)]
#[tokio::test]
async fn the_apps_own_folder_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let folder = tempfile::Builder::new()
        .prefix("ja")
        .tempdir()
        .expect("temporary folder");
    let own = folder.path().join("jutsu-studio");
    let endpoint = AgentEndpoint {
        name: own.join(socket::FILE_NAME),
        folder: own.clone(),
        is_override: false,
        owns_folder: true,
    };
    let server = AgentServer::new(endpoint, Arc::new(Handler::allowing(true)));
    server.start().expect("listening");
    assert_eq!(
        fs::metadata(&own).expect("folder").permissions().mode() & 0o777,
        0o700
    );
    server.stop();
    assert!(
        fs::metadata(own.join(socket::FILE_NAME)).is_err(),
        "stop removes the socket file"
    );
}

#[tokio::test]
async fn a_socket_path_too_long_is_refused_by_both_ends() {
    let endpoint = AgentEndpoint::at(PathBuf::from(format!(
        "/tmp/{}",
        "x".repeat(socket::MAX_PATH_LENGTH)
    )));
    assert!(!endpoint.fits());
    let server = AgentServer::new(endpoint.clone(), Arc::new(Handler::allowing(true)));
    assert!(matches!(server.start(), Err(Error::PathTooLong)));
    assert!(matches!(
        AgentClient::connect(&endpoint, Duration::from_secs(1)).await,
        Err(Error::PathTooLong)
    ));
    let refusal = AgentError::socket_path_too_long(&endpoint.name);
    assert_eq!(refusal.code, ErrorCode::SOCKET_PATH_TOO_LONG);
    assert!(refusal.next.is_some_and(|next| next.contains(socket::VARIABLE)));
}

#[tokio::test]
async fn nothing_listening_is_not_listening() {
    let (_folder, endpoint) = temporary_socket();
    let connected = AgentClient::connect(&endpoint, Duration::from_secs(1)).await;
    assert!(matches!(connected, Err(Error::NotListening(_))));
}

// Inbox

fn file(folder: &TempDir, name: &str, text: &str) -> PathBuf {
    let path = folder.path().join(name);
    fs::write(&path, text).expect("file");
    path
}

fn upload(name: &str, size: i64) -> UploadCommand {
    UploadCommand {
        name: name.to_owned(),
        size,
        request: None,
    }
}

fn listing(folder: &Path) -> Vec<String> {
    fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn uploads_of_one_name_never_replace_each_other() {
    let (folder, server, _handler) = started(Handler::allowing(true));
    let first = file(&folder, "first.mov", "first");
    let second = file(&folder, "second.mov", "second, longer");
    let mut client = client(&server).await;
    let answers = [
        client
            .upload(&upload("clip.mov", 5), &first, None, None)
            .await
            .expect("answer"),
        client
            .upload(&upload("clip.mov", 14), &second, None, None)
            .await
            .expect("answer"),
    ];
    let paths: Vec<String> = answers
        .iter()
        .map(|answer| answer.uploaded.clone().expect("uploaded"))
        .collect();
    assert_ne!(paths[0], paths[1]);
    let inbox = server.inbox.folder.to_string_lossy().into_owned();
    assert!(paths.iter().all(|path| path.starts_with(&inbox)));
    assert_eq!(fs::read_to_string(&paths[0]).expect("first"), "first");
    assert_eq!(fs::read_to_string(&paths[1]).expect("second"), "second, longer");
    // Nothing half-written is left beside them.
    let staged = Path::new(&paths[0]).parent().expect("folder");
    assert_eq!(listing(staged), ["clip.mov"]);
}

#[tokio::test]
async fn a_refused_upload_reads_no_bytes_and_stages_nothing() {
    let taken = AgentError::new(ErrorCode::REQUEST_CONFLICT, "taken", false, None);
    let (folder, server, handler) = started(Handler {
        refuse_uploads: Some(taken),
        ..Handler::allowing(true)
    });
    let source = file(&folder, "source.mov", "bytes");
    let refused = client(&server)
        .await
        .upload(&upload("clip.mov", 5), &source, None, None)
        .await
        .expect("answer");
    assert_eq!(
        refused.error.map(|error| error.code),
        Some(ErrorCode::REQUEST_CONFLICT)
    );
    handler.allowed.store(false, Ordering::SeqCst);
    let off = client(&server)
        .await
        .upload(&upload("clip.mov", 5), &source, None, None)
        .await
        .expect("answer");
    assert_eq!(off.error, Some(AgentError::agents_off()));
    assert_eq!(listing(&server.inbox.folder).len(), 0);
}

/// An upload must leave about 2 GB free on the volume and keep the inbox within 8 GiB; an unknown free space refuses
/// nothing.
#[test]
fn an_upload_needs_room_on_the_volume_and_in_the_inbox() {
    let size = 1 << 30;
    let room = inbox::MIN_FREE_SPACE + size;
    assert_eq!(AgentInbox::room_problem(size, Some(room), 0), None);
    assert_eq!(AgentInbox::room_problem(size, None, 0), None);
    assert!(AgentInbox::room_problem(size, Some(room - 1), 0).is_some());
    assert_eq!(
        AgentInbox::room_problem(size, Some(room), inbox::MAX_STORED_SIZE - size),
        None
    );
    assert!(AgentInbox::room_problem(size, Some(room), inbox::MAX_STORED_SIZE - size + 1).is_some());
}

/// A refusal for room says how much is free and how much the file needs, the same in every locale.
#[test]
fn a_refusal_for_room_gives_the_numbers() {
    let volume = AgentInbox::room_problem(1_200_000_000, Some(2_310_000_000), 0).expect("refused");
    assert!(volume.starts_with("The computer has"), "{volume}");
    assert!(
        volume.contains("2.31 GB free") && volume.contains("needs 1.20 GB"),
        "{volume}"
    );
    assert!(
        volume.contains("2.00 GB left free") && volume.contains("free up 0.89 GB"),
        "{volume}"
    );
    let stored =
        AgentInbox::room_problem(1 << 30, None, inbox::MAX_STORED_SIZE - (1 << 29)).expect("refused");
    assert!(
        stored.contains("take 7.50 GiB") && stored.contains("1.00 GiB file") && stored.contains("8.00 GiB")
    );
}

#[tokio::test]
async fn an_upload_without_room_is_refused_for_good() {
    let (folder, server, _handler) = started(Handler {
        free_space: Some(1 << 30),
        ..Handler::allowing(true)
    });
    let source = file(&folder, "source.mov", "bytes");
    let refused = client(&server)
        .await
        .upload(&upload("clip.mov", 5), &source, None, None)
        .await
        .expect("answer");
    let refusal = refused.error.expect("refused");
    assert_eq!(refusal.code, ErrorCode::FILE_REFUSED);
    assert!(!refusal.retryable);
    assert_eq!(listing(&server.inbox.folder).len(), 0);
}

#[test]
fn the_inbox_counts_what_its_uploads_hold() {
    let (_folder, endpoint) = temporary_socket();
    let inbox = AgentInbox::new(endpoint.inbox());
    assert_eq!(inbox.stored_size(), 0);
    fs::write(inbox.make_upload_folder().expect("folder").join("a.png"), [0; 3]).expect("write");
    fs::write(
        inbox
            .make_upload_folder()
            .expect("folder")
            .join(inbox::PARTIAL_NAME),
        [0; 5],
    )
    .expect("write");
    assert_eq!(inbox.stored_size(), 8);
}

#[test]
fn placing_a_file_never_replaces_one() {
    let (_folder, endpoint) = temporary_socket();
    let inbox = AgentInbox::new(endpoint.inbox());
    let folder = inbox.make_upload_folder().expect("folder");
    assert_ne!(inbox.make_upload_folder().expect("folder"), folder);
    let staged = folder.join("clip.mov");
    fs::write(&staged, "kept").expect("write");
    let partial = folder.join(inbox::PARTIAL_NAME);
    fs::write(&partial, "new").expect("write");
    let placed = AgentInbox::place(&partial, &staged);
    assert_eq!(
        placed.map_err(|error| error.kind()),
        Err(std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(fs::read_to_string(&staged).expect("staged"), "kept");
}

/// The refusals for room name the computer, which may not be a Mac.
#[test]
fn refusals_for_room_name_the_computer() {
    let full = crate::AgentError::no_room("clip.mov", "no room");
    assert!(
        full.next
            .as_deref()
            .is_some_and(|next| next.contains("free space on the computer"))
    );
    let unstored = crate::AgentError::could_not_store("clip.mov", &std::io::Error::other("disk"));
    assert!(
        unstored
            .next
            .as_deref()
            .is_some_and(|next| next.contains("the computer has room"))
    );
}
