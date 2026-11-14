//! The app's end of the agent socket. It listens from launch, and reads Allow agents again (`AgentHandler::
//! agents_allowed`) for every command and every upload, so turning the setting off refuses the next command at once,
//! on a connection opened before too, with `agents_off`. Turning it off stops nothing an agent started.

use crate::{
    ANSWER_DEADLINE, AgentCommand, AgentEndpoint, AgentError, AgentInbox, AgentLineBuffer, AgentRequest,
    AgentResponse, Error, ErrorCode, UploadCommand, inbox, wire,
};
use async_trait::async_trait;
use interprocess::local_socket::{
    GenericFilePath, ListenerOptions, ToFsName,
    tokio::{Listener, Stream, prelude::*},
};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

/// What the app does with the commands the server accepts. The server knows nothing of the app beyond this.
#[async_trait]
pub trait AgentHandler: Send + Sync + 'static {
    /// Settings › Agents › Allow agents, read for every command and every upload.
    fn agents_allowed(&self) -> bool;

    /// Runs before an upload's bytes are read, after Allow agents and the upload's own fields were checked: a refusal
    /// leaves the bytes unread and nothing in the inbox. A command whose request ID is already taken by other
    /// parameters is refused here.
    fn check_upload(&self, _upload: &UploadCommand, _client: Option<&str>) -> Result<(), AgentError> {
        Ok(())
    }

    /// Runs one command the app has accepted; the server fills in the response's `id`.
    async fn handle(&self, request: AgentRequest) -> Result<AgentResponse, AgentError>;

    /// The bytes free on the volume of `path`, `None` when unknown; an upload must leave `inbox::MIN_FREE_SPACE`.
    fn free_space(&self, _path: &Path) -> Option<i64> {
        None
    }

    /// Called with `true` when the first connection opens and `false` when the last one closes, so an app that the
    /// system would throttle in the background can stay awake while it serves an agent.
    fn hold_activity(&self, _serving: bool) {}
}

/// How long writing an answer may take before the connection is given up.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// The most of an upload read into memory at once.
const UPLOAD_CHUNK: usize = 1 << 20;

struct Shared {
    handler: Arc<dyn AgentHandler>,
    inbox: AgentInbox,
    answer_deadline: Duration,
    connections: Mutex<usize>,
}

impl Shared {
    fn connections(&self) -> MutexGuard<'_, usize> {
        self.connections.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn opened(&self) {
        let mut count = self.connections();
        *count += 1;
        if *count == 1 {
            self.handler.hold_activity(true);
        }
    }

    fn closed(&self) {
        let mut count = self.connections();
        if *count == 0 {
            return;
        }
        *count -= 1;
        if *count == 0 {
            self.handler.hold_activity(false);
        }
    }
}

/// The listener: started inside a tokio runtime, it accepts connections until `stop`.
pub struct AgentServer {
    pub endpoint: AgentEndpoint,
    pub inbox: AgentInbox,
    shared: Arc<Shared>,
    running: Mutex<Option<CancellationToken>>,
}

impl AgentServer {
    pub fn new(endpoint: AgentEndpoint, handler: Arc<dyn AgentHandler>) -> Self {
        Self::with_answer_deadline(endpoint, handler, ANSWER_DEADLINE)
    }

    /// A server whose answers wait for their work at most `answer_deadline` (`ANSWER_DEADLINE`).
    pub fn with_answer_deadline(
        endpoint: AgentEndpoint,
        handler: Arc<dyn AgentHandler>,
        answer_deadline: Duration,
    ) -> Self {
        let inbox = AgentInbox::new(endpoint.inbox());
        let shared = Arc::new(Shared {
            handler,
            inbox: inbox.clone(),
            answer_deadline,
            connections: Mutex::new(0),
        });
        Self {
            endpoint,
            inbox,
            shared,
            running: Mutex::new(None),
        }
    }

    fn running(&self) -> MutexGuard<'_, Option<CancellationToken>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_listening(&self) -> bool {
        self.running().is_some()
    }

    /// Listens on the endpoint, creating its folder. Must be called inside a tokio runtime.
    pub fn start(&self) -> Result<(), Error> {
        let mut running = self.running();
        if running.is_some() {
            return Ok(());
        }
        if !self.endpoint.fits() {
            return Err(Error::PathTooLong);
        }
        prepare_folder(&self.endpoint).map_err(Error::Listen)?;
        let listener = listen(&self.endpoint).map_err(Error::Listen)?;
        let token = CancellationToken::new();
        tokio::spawn(accept(listener, self.shared.clone(), token.clone()));
        *running = Some(token);
        Ok(())
    }

    /// Stops listening, closes open connections and removes the socket file.
    pub fn stop(&self) {
        let Some(token) = self.running().take() else {
            return;
        };
        token.cancel();
        let mut count = self.shared.connections();
        if *count > 0 {
            *count = 0;
            self.shared.handler.hold_activity(false);
        }
        drop(count);
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.endpoint.name);
        }
    }

    /// The answer to one request line, for any command but `upload`, whose bytes follow its line on the socket.
    pub async fn response(&self, line: &[u8]) -> AgentResponse {
        response(&self.shared, line).await
    }
}

impl Drop for AgentServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The socket's folder, made private when it is the app's own.
fn prepare_folder(endpoint: &AgentEndpoint) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if let Some(folder) = endpoint.name.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(folder)?;
            if endpoint.owns_folder {
                std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    #[cfg(windows)]
    {
        std::fs::create_dir_all(&endpoint.folder)?;
    }
    Ok(())
}

/// A listener on the endpoint, readable and writable by this user only.
#[cfg(unix)]
fn listen(endpoint: &AgentEndpoint) -> io::Result<Listener> {
    use interprocess::os::unix::local_socket::ListenerOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let options = || -> io::Result<ListenerOptions<'_>> {
        Ok(ListenerOptions::new()
            .name(endpoint.name.as_path().to_fs_name::<GenericFilePath>()?)
            .reclaim_name(false)
            .try_overwrite(true))
    };
    // The mode is set before the socket listens where the system allows it; macOS doesn't, and there the socket is
    // narrowed right after, inside a folder only this user can enter.
    match options()?.mode(0o600).create_tokio() {
        Err(error) if error.kind() == io::ErrorKind::Unsupported || error.raw_os_error() == Some(22) => {
            let listener = options()?.create_tokio()?;
            std::fs::set_permissions(&endpoint.name, std::fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }
        result => result,
    }
}

/// A named pipe open to its owner only: the DACL grants all access to OWNER RIGHTS and nothing to anyone else.
#[cfg(windows)]
fn listen(endpoint: &AgentEndpoint) -> io::Result<Listener> {
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)")
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let descriptor = SecurityDescriptor::deserialize(&sddl)?;
    ListenerOptions::new()
        .name(endpoint.name.as_path().to_fs_name::<GenericFilePath>()?)
        .security_descriptor(descriptor)
        .create_tokio()
}

async fn accept(listener: Listener, shared: Arc<Shared>, token: CancellationToken) {
    loop {
        let stream = tokio::select! {
            () = token.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        match stream {
            Ok(stream) => {
                shared.opened();
                let (shared, token) = (shared.clone(), token.clone());
                tokio::spawn(async move {
                    tokio::select! {
                        () = token.cancelled() => {}
                        () = serve(stream, &shared) => {}
                    }
                    shared.closed();
                });
            }
            Err(error) => {
                tracing::warn!(%error, "the agent socket couldn't accept a connection");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// The answer to one line, refused while agents are off.
async fn response(shared: &Shared, line: &[u8]) -> AgentResponse {
    let (id, request) = wire::request(line);
    let answer = async {
        if !shared.handler.agents_allowed() {
            return Err(AgentError::agents_off());
        }
        let request = request?;
        if matches!(request.command, AgentCommand::Upload(_)) {
            return Err(AgentError::bad_request(
                "upload is read with the bytes that follow it",
            ));
        }
        answer(shared, request).await
    };
    match answer.await {
        Ok(mut response) => {
            response.id = id;
            response
        }
        Err(error) => AgentResponse::failure(id, error),
    }
}

/// The handler's answer, or `still_working` once the answer deadline passes first. The work runs in a task of its
/// own and goes on either way: nothing a command started is stopped because its answer came too late.
async fn answer(shared: &Shared, request: AgentRequest) -> Result<AgentResponse, AgentError> {
    let name = request.command.name();
    let handler = shared.handler.clone();
    let work = tokio::spawn(async move { handler.handle(request).await });
    match tokio::time::timeout(shared.answer_deadline, work).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(AgentError::new(
            ErrorCode::FAILED,
            "Jutsu Studio failed to answer.",
            true,
            None,
        )),
        Err(_) => Err(AgentError::still_working(name)),
    }
}

/// Answers each line in order, and reads on only once it is answered. A refused upload closes the connection: the
/// bytes that may follow can't be told from the next request.
async fn serve(mut stream: Stream, shared: &Shared) {
    let mut buffer = AgentLineBuffer::new();
    let mut chunk = vec![0; 1 << 16];
    loop {
        let Some(line) = buffer.next_line() else {
            if buffer.is_overlong() {
                return;
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(count) => buffer.push(&chunk[..count]),
            }
            continue;
        };
        let (id, request) = wire::request(&line);
        let Ok(AgentRequest {
            command: AgentCommand::Upload(upload),
            client,
            ..
        }) = request
        else {
            if !send(&mut stream, &response(shared, &line).await).await {
                return;
            }
            continue;
        };
        let upload = (upload, client);
        match receive(shared, &upload.0, upload.1.as_deref(), &mut buffer, &mut stream).await {
            Upload::Stored(path) => {
                let answer = AgentResponse {
                    id,
                    uploaded: Some(path),
                    ..AgentResponse::default()
                };
                if !send(&mut stream, &answer).await {
                    return;
                }
            }
            Upload::Refused(error) => {
                send(&mut stream, &AgentResponse::failure(id, error)).await;
                return;
            }
            Upload::Dropped => return,
        }
    }
}

/// Writes `response` as one line; false when the connection is gone or the write took too long.
async fn send(stream: &mut Stream, response: &AgentResponse) -> bool {
    let Ok(line) = wire::line(response) else {
        return false;
    };
    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, stream.write_all(&line)).await,
        Ok(Ok(()))
    )
}

enum Upload {
    /// The file's path in the inbox.
    Stored(String),
    Refused(AgentError),
    /// The client went away before all the bytes came.
    Dropped,
}

/// `upload.size` bytes from `buffer` and then the connection, through a hidden partial file in a new folder of the
/// inbox, moved to its name only once complete, so nothing half-written is ever named.
async fn receive(
    shared: &Shared,
    upload: &UploadCommand,
    client: Option<&str>,
    buffer: &mut AgentLineBuffer,
    stream: &mut Stream,
) -> Upload {
    let checked = (|| {
        if !shared.handler.agents_allowed() {
            return Err(AgentError::agents_off());
        }
        if let Some(problem) = UploadCommand::problem(&upload.name, upload.size) {
            return Err(AgentError::bad_request(problem));
        }
        shared.handler.check_upload(upload, client)?;
        let free = shared.handler.free_space(&shared.inbox.folder);
        match AgentInbox::room_problem(upload.size, free, shared.inbox.stored_size()) {
            Some(problem) => Err(AgentError::no_room(&upload.name, &problem)),
            None => Ok(()),
        }
    })();
    if let Err(error) = checked {
        return Upload::Refused(error);
    }
    let folder = match shared.inbox.make_upload_folder() {
        Ok(folder) => folder,
        Err(error) => return Upload::Refused(AgentError::could_not_store(&upload.name, &error)),
    };
    let partial = folder.join(inbox::PARTIAL_NAME);
    let destination = folder.join(&upload.name);
    let file = match tokio::fs::File::create(&partial).await {
        Ok(file) => file,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&folder);
            return Upload::Refused(AgentError::could_not_store(&upload.name, &error));
        }
    };
    let Some(written) = copy(upload.size, buffer, stream, file).await else {
        let _ = std::fs::remove_dir_all(&folder);
        return Upload::Dropped;
    };
    if let Err(error) = written.and_then(|()| AgentInbox::place(&partial, &destination)) {
        let _ = std::fs::remove_dir_all(&folder);
        return Upload::Refused(AgentError::could_not_store(&upload.name, &error));
    }
    Upload::Stored(destination.to_string_lossy().into_owned())
}

/// Writes the next `size` bytes to `file`: `None` when the client went away first. After a failed write the rest is
/// still read, so the answer comes once the client has sent everything.
async fn copy(
    size: i64,
    buffer: &mut AgentLineBuffer,
    stream: &mut Stream,
    mut file: tokio::fs::File,
) -> Option<io::Result<()>> {
    let mut remaining = u64::try_from(size).unwrap_or(0);
    let mut result = Ok(());
    let mut incoming = vec![0; 1 << 16];
    while remaining > 0 {
        let wanted = usize::try_from(remaining)
            .unwrap_or(UPLOAD_CHUNK)
            .min(UPLOAD_CHUNK);
        let mut chunk = buffer.take(wanted);
        if chunk.is_empty() {
            match stream.read(&mut incoming).await {
                Ok(0) | Err(_) => return None,
                Ok(count) => buffer.push(&incoming[..count]),
            }
            chunk = buffer.take(wanted);
        }
        remaining -= chunk.len() as u64;
        if result.is_ok() {
            result = file.write_all(&chunk).await;
        }
    }
    if result.is_ok() {
        result = file.flush().await;
    }
    Some(result)
}
