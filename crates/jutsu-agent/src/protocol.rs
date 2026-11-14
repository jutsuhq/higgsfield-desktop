//! The commands an agent can send the app, shared by the app and the `jutsu` helper. One JSON object per line each
//! way over a local socket: a request, then its response, in order.
//!
//! ```text
//! → {"version":1,"id":"1","client":"claude-code","command":"show","page":"queue"}
//! ← {"id":"1","shown":{"page":"queue"}}
//! ```

use crate::{
    AgentActed, AgentCapabilities, AgentCreated, AgentDelivery, AgentDownloads, AgentElementsPage,
    AgentError, AgentExported, AgentFrames, AgentJobs, AgentLibraryDetailsPage, AgentLibraryPage,
    AgentMarked, AgentQuote, AgentShown, AgentSubtitles, CapabilitiesCommand, CreateElementCommand,
    CreateImageCommand, CreateVideoCommand, DeliverySetCommand, ElementsCommand, EnhanceCommand,
    ExportCommand, FramesCommand, JobActionCommand, JobsCommand, LibraryDownloadCommand, LibraryGetCommand,
    LibraryMarkCommand, LibrarySearchCommand, QuoteCommand, ShowCommand, SubtitlesCommand, UploadCommand,
    json,
};
use serde::{Deserialize, Serialize, Serializer, ser::Error as _};
use serde_json::{Map, Value};
use std::time::Duration;

/// Raised only for a change an older peer would misread; new commands and new optional fields don't need it.
pub const PROTOCOL_VERSION: i64 = 1;
/// A request line longer than this is refused rather than buffered. An upload's bytes follow its line and don't count.
pub const MAX_LINE_LENGTH: usize = 64 * 1024;
/// The app answers every command within this, under the minute MCP clients usually give a tool call.
pub const ANSWER_DEADLINE: Duration = Duration::from_secs(55);
/// How long the helper waits on the socket for each write and for the answer: the answer deadline and a margin.
pub const HELPER_TIMEOUT: Duration = Duration::from_secs(60);
/// Passed to the app when the helper starts it because a call found it not running: the app then starts with no
/// window and doesn't take the focus, and `show` opens the window when the person is to see something.
pub const HELPER_LAUNCH_ARGUMENT: &str = "--started-by-agent";

/// How long a command that already took `elapsed` may still wait for `seconds` and answer in time: its wait, cut to
/// what is left of the answer deadline less a margin for the answer itself.
pub fn wait_seconds(seconds: i64, elapsed: Duration) -> i64 {
    let left = ANSWER_DEADLINE
        .saturating_sub(Duration::from_secs(5))
        .saturating_sub(elapsed)
        .as_secs();
    seconds.min(i64::try_from(left).unwrap_or(i64::MAX)).max(0)
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentCommand {
    /// What the app can do now: the connection, credits, the agent budget and the tool groups.
    Capabilities(CapabilitiesCommand),
    /// Opens a page or a job in the app window and brings the window forward.
    Show(ShowCommand),
    /// A file for the inbox, its bytes after the line. Only the helper sends it, for a tool that takes a file.
    Upload(UploadCommand),
    Quote(QuoteCommand),
    CreateVideo(CreateVideoCommand),
    Jobs(JobsCommand),
    JobAction(JobActionCommand),
    CreateImage(CreateImageCommand),
    LibrarySearch(LibrarySearchCommand),
    LibraryGet(LibraryGetCommand),
    Elements(ElementsCommand),
    CreateElement(CreateElementCommand),
    LibraryDownload(LibraryDownloadCommand),
    LibraryMark(LibraryMarkCommand),
    Enhance(EnhanceCommand),
    DeliverySet(DeliverySetCommand),
    Export(ExportCommand),
    Frames(FramesCommand),
    Subtitles(SubtitlesCommand),
}

impl AgentCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Capabilities(_) => "capabilities",
            Self::Show(_) => "show",
            Self::Upload(_) => "upload",
            Self::Quote(_) => "quote",
            Self::CreateVideo(_) => "create_video",
            Self::Jobs(_) => "jobs",
            Self::JobAction(_) => "job_action",
            Self::CreateImage(_) => "create_image",
            Self::LibrarySearch(_) => "library_search",
            Self::LibraryGet(_) => "library_get",
            Self::Elements(_) => "elements",
            Self::CreateElement(_) => "create_element",
            Self::LibraryDownload(_) => "library_download",
            Self::LibraryMark(_) => "library_mark",
            Self::Enhance(_) => "enhance",
            Self::DeliverySet(_) => "delivery_set",
            Self::Export(_) => "export",
            Self::Frames(_) => "frames",
            Self::Subtitles(_) => "subtitles",
        }
    }

    /// The command's own fields, as they sit next to `version`, `id` and `command`.
    fn fields(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Capabilities(fields) => json::to_value(fields),
            Self::Show(fields) => json::to_value(fields),
            Self::Upload(fields) => json::to_value(fields),
            Self::Quote(fields) => json::to_value(fields),
            Self::CreateVideo(fields) => json::to_value(fields),
            Self::Jobs(fields) => json::to_value(fields),
            Self::JobAction(fields) => json::to_value(fields),
            Self::CreateImage(fields) => json::to_value(fields),
            Self::LibrarySearch(fields) => json::to_value(fields),
            Self::LibraryGet(fields) => json::to_value(fields),
            Self::Elements(fields) => json::to_value(fields),
            Self::CreateElement(fields) => json::to_value(fields),
            Self::LibraryDownload(fields) => json::to_value(fields),
            Self::LibraryMark(fields) => json::to_value(fields),
            Self::Enhance(fields) => json::to_value(fields),
            Self::DeliverySet(fields) => json::to_value(fields),
            Self::Export(fields) => json::to_value(fields),
            Self::Frames(fields) => json::to_value(fields),
            Self::Subtitles(fields) => json::to_value(fields),
        }
    }
}

/// A request: flat, with `command` naming the rest of the keys, which is what a person types into a socket too.
/// Read with `wire::request`, which refuses a malformed one with the error to answer it with.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRequest {
    pub version: i64,
    /// Echoed in the response, so a client can match them up.
    pub id: Option<String>,
    /// The agent's name as its MCP client reports it (`clientInfo`), for the person to see where a job came from.
    pub client: Option<String>,
    pub command: AgentCommand,
}

impl AgentRequest {
    pub fn new(command: AgentCommand) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id: None,
            client: None,
            command,
        }
    }

    /// The request as one flat JSON object.
    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        let mut object = match self.command.fields()? {
            Value::Object(fields) => fields,
            _ => Map::new(),
        };
        object.insert("version".to_owned(), Value::from(self.version));
        if let Some(id) = &self.id {
            object.insert("id".to_owned(), Value::from(id.as_str()));
        }
        if let Some(client) = &self.client {
            object.insert("client".to_owned(), Value::from(client.as_str()));
        }
        object.insert("command".to_owned(), Value::from(self.command.name()));
        Ok(Value::Object(object))
    }
}

impl Serialize for AgentRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().map_err(S::Error::custom)?.serialize(serializer)
    }
}

/// Exactly one of the result fields or `error` is set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: Option<String>,
    pub capabilities: Option<AgentCapabilities>,
    /// show: what the window now shows.
    pub shown: Option<AgentShown>,
    /// upload: where the app put the file, a path inside its inbox.
    pub uploaded: Option<String>,
    pub quote: Option<AgentQuote>,
    /// `create_video`, `create_image` and enhance: the jobs the request queued; `create_element`: the element.
    pub created: Option<AgentCreated>,
    pub jobs: Option<AgentJobs>,
    /// `job_action`: what was done.
    pub acted: Option<AgentActed>,
    /// `library_search`: a page of results.
    pub library: Option<AgentLibraryPage>,
    /// `library_get`: the items in full.
    pub details: Option<AgentLibraryDetailsPage>,
    pub downloads: Option<AgentDownloads>,
    pub elements: Option<AgentElementsPage>,
    /// `library_mark`: the items as they are now.
    pub marked: Option<AgentMarked>,
    pub delivery: Option<AgentDelivery>,
    pub exported: Option<AgentExported>,
    pub frames: Option<AgentFrames>,
    /// subtitles get and export; make answers with `created`.
    pub subtitles: Option<AgentSubtitles>,
    pub error: Option<AgentError>,
}

impl AgentResponse {
    /// The answer that refuses request `id` with `error`.
    pub fn failure(id: Option<String>, error: AgentError) -> Self {
        Self {
            id,
            error: Some(error),
            ..Self::default()
        }
    }
}
