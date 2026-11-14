//! A refused or failed command (`AgentError`), and this crate's own errors.

use crate::{json, socket};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt, io, path::Path};

/// A string, not an enum, so a client still decodes a code added after it was built.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorCode(pub Cow<'static, str>);

impl ErrorCode {
    pub const AGENTS_OFF: Self = Self::named("agents_off");
    pub const UNSUPPORTED_VERSION: Self = Self::named("unsupported_version");
    pub const UNKNOWN_COMMAND: Self = Self::named("unknown_command");
    pub const BAD_REQUEST: Self = Self::named("bad_request");
    pub const NOT_FOUND: Self = Self::named("not_found");
    /// A file the app can't take, with why.
    pub const FILE_REFUSED: Self = Self::named("file_refused");
    /// The socket's path is longer than a local socket address holds (`socket::MAX_PATH_LENGTH`).
    pub const SOCKET_PATH_TOO_LONG: Self = Self::named("socket_path_too_long");
    /// The same request ID came with other parameters.
    pub const REQUEST_CONFLICT: Self = Self::named("request_conflict");
    /// The request ID was already used with these parameters: its files aren't needed again, the job exists.
    pub const REQUEST_KNOWN: Self = Self::named("request_known");
    /// Nothing is connected to Higgsfield.
    pub const NOT_CONNECTED: Self = Self::named("not_connected");
    /// The request breaks the model's rules or limits; the message says which.
    pub const INVALID_REQUEST: Self = Self::named("invalid_request");
    /// Over the person's own daily spend cap: only they can raise it or start the job.
    pub const OVER_CAP: Self = Self::named("over_cap");
    /// Only the person can do this, in the app.
    pub const NEEDS_PERSON: Self = Self::named("needs_person");
    /// The job is in a state the action doesn't apply to.
    pub const NOT_APPLICABLE: Self = Self::named("not_applicable");
    /// The app failed to do it; the message says why.
    pub const FAILED: Self = Self::named("failed");
    /// Higgsfield prices the operation only after it has run, so no agent can stay within a `max_cost` for it.
    pub const PRICED_AFTER_RUN: Self = Self::named("priced_after_run");
    /// This computer can't run the engine.
    pub const ENGINE_UNAVAILABLE: Self = Self::named("engine_unavailable");
    /// The item has a run going already; the message names it.
    pub const BUSY: Self = Self::named("busy");
    /// The command's work took longer than the app answers within (`ANSWER_DEADLINE`); it goes on.
    pub const STILL_WORKING: Self = Self::named("still_working");

    /// A code by its raw value.
    pub const fn named(raw: &'static str) -> Self {
        Self(Cow::Borrowed(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A refused or failed command. `code` is stable for clients to branch on, `message` is for the person, `next` says
/// what to do instead, and `retryable` whether sending the same request again later can succeed. Agents read these as
/// tool results, so the text is English and written for them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct AgentError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub next: Option<String>,
}

impl AgentError {
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool, next: Option<&str>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            next: next.map(str::to_owned),
        }
    }

    /// Retryable: the person can turn it on, and the same request then works.
    pub fn agents_off() -> Self {
        Self::new(
            ErrorCode::AGENTS_OFF,
            "Agents are turned off in Jutsu Studio.",
            true,
            Some(
                "Ask the person to turn on Settings › Agents › Allow agents in Jutsu Studio, then send the request again.",
            ),
        )
    }

    pub fn unsupported_version(version: i64) -> Self {
        Self::new(
            ErrorCode::UNSUPPORTED_VERSION,
            format!(
                "Protocol version {version} is not supported; this app speaks {}.",
                crate::PROTOCOL_VERSION
            ),
            false,
            Some("Use the jutsu helper inside this Jutsu Studio.app."),
        )
    }

    pub fn unknown_command(name: &str) -> Self {
        Self::new(
            ErrorCode::UNKNOWN_COMMAND,
            format!("Unknown command '{}'.", crate::names::prefix(name, 64)),
            false,
            Some("Send capabilities to see what this app supports."),
        )
    }

    pub fn bad_request(reason: impl fmt::Display) -> Self {
        Self::new(
            ErrorCode::BAD_REQUEST,
            format!("Malformed request: {reason}."),
            false,
            Some("Fix the arguments as the message says and call the tool again."),
        )
    }

    pub fn invalid_request(reason: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::INVALID_REQUEST,
            reason,
            false,
            Some(
                "Change the request as the message says; capabilities with models true lists each model's limits.",
            ),
        )
    }

    /// Nothing is stopped: a request with a `request_id` sent again returns what it made, once it has.
    pub fn still_working(command: &str) -> Self {
        Self::new(
            ErrorCode::STILL_WORKING,
            format!("Jutsu Studio is still working on {command}; it goes on without this answer."),
            true,
            Some(
                "Send the same request again in a moment (the same request_id returns what it made), or call jobs.",
            ),
        )
    }

    pub fn request_conflict() -> Self {
        Self::new(
            ErrorCode::REQUEST_CONFLICT,
            "This request_id was already used with other parameters.",
            false,
            Some(
                "Use a new request_id for a different request, or send exactly the same parameters to get its job.",
            ),
        )
    }

    pub fn socket_path_too_long(socket: &Path) -> Self {
        let path = socket.to_string_lossy();
        Self::new(
            ErrorCode::SOCKET_PATH_TOO_LONG,
            format!(
                "The agent socket's path is {} bytes, more than the {} a Unix socket takes: {path}",
                path.len(),
                socket::MAX_PATH_LENGTH
            ),
            false,
            Some(&format!(
                "Agents need a home folder with a shorter path. For development, point {} at a shorter path.",
                socket::VARIABLE
            )),
        )
    }

    /// Not retried as it is: the room comes back only once files are removed or the waiting jobs take theirs.
    pub fn no_room(name: &str, problem: &str) -> Self {
        Self::new(
            ErrorCode::FILE_REFUSED,
            format!("Jutsu Studio didn't take {name}: {problem}"),
            false,
            Some(
                "Ask the person to free space on the computer or let queued jobs start, or send a smaller file.",
            ),
        )
    }

    pub fn could_not_store(name: &str, error: &io::Error) -> Self {
        Self::new(
            ErrorCode::FILE_REFUSED,
            format!("Jutsu Studio couldn't store {name}: {error}"),
            true,
            Some("Check that the computer has room for it, then send the request again."),
        )
    }
}

/// Carries an `AgentError` through serde's decoding, so a check made while a field decodes refuses the request
/// with its own words rather than a generic `bad <field>`.
pub(crate) struct Refusal(pub AgentError);

/// What starts a decoding error's message that carries an `AgentError` as JSON.
pub(crate) const REFUSAL_MARKER: &str = "\u{1}agent-error:";

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded = json::to_value(&self.0)
            .map(|value| json::write(&value))
            .unwrap_or_default();
        write!(formatter, "{REFUSAL_MARKER}{encoded}")
    }
}

/// The socket's own failures, on either end.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Nothing listens there: the app isn't running, or is still starting.
    #[error("nothing listens on the agent socket: {0}")]
    NotListening(io::Error),
    /// The app closed the connection.
    #[error("Jutsu Studio closed the connection")]
    Closed,
    /// A write or the wait for the answer took longer than the timeout.
    #[error("Jutsu Studio didn't answer in time")]
    TimedOut,
    /// Longer than `socket::MAX_PATH_LENGTH` (`AgentError::socket_path_too_long` says it to an agent).
    #[error("the agent socket's path is too long")]
    PathTooLong,
    /// The file to upload ended before the size it was sent with.
    #[error("the file changed while it was sent")]
    FileChanged,
    /// The file to upload couldn't be read.
    #[error("couldn't read the file: {0}")]
    File(io::Error),
    /// The answer isn't one the protocol knows, or a request couldn't be written.
    #[error("the message isn't one the protocol knows: {0}")]
    Malformed(serde_json::Error),
    /// The listener couldn't start.
    #[error("couldn't listen on the agent socket: {0}")]
    Listen(io::Error),
}
