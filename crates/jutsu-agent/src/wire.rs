//! Newline-delimited JSON: compact JSON never contains a raw newline, so a line is always one whole message.

use crate::{
    AgentCommand, AgentError, AgentRequest, MAX_LINE_LENGTH, PROTOCOL_VERSION, error::REFUSAL_MARKER, json,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// `message` as one line of compact JSON with sorted keys, its newline included.
pub fn line<T: Serialize + ?Sized>(message: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut text = json::write(&json::to_value(message)?);
    text.push('\n');
    Ok(text.into_bytes())
}

/// The value a line holds, read as `line` wrote it.
pub fn decode<T: DeserializeOwned>(line: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(line)
}

/// The request on `line`, or the error to answer it with, and the request's `id` whenever it could be read.
pub fn request(line: &[u8]) -> (Option<String>, Result<AgentRequest, AgentError>) {
    let envelope = || AgentError::bad_request("not a JSON object with a string id");
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(line) else {
        return (None, Err(envelope()));
    };
    let id = match object.get("id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => return (None, Err(envelope())),
    };
    // Checked before the command, so a newer client gets the version error rather than an unknown command.
    let version = match object.get("version") {
        None | Some(Value::Null) => return (id, Err(AgentError::bad_request("no version"))),
        Some(Value::Number(number)) => match whole(number) {
            Some(version) => version,
            None => return (None, Err(envelope())),
        },
        Some(_) => return (None, Err(envelope())),
    };
    if version != PROTOCOL_VERSION {
        return (id, Err(AgentError::unsupported_version(version)));
    }
    let client = match object.get("client") {
        None | Some(Value::Null) => None,
        Some(Value::String(client)) => Some(client.clone()),
        Some(_) => return (id, Err(AgentError::bad_request("bad client"))),
    };
    let name = match object.get("command") {
        None | Some(Value::Null) => return (id, Err(AgentError::bad_request("no command"))),
        Some(Value::String(name)) => name.clone(),
        Some(_) => return (id, Err(AgentError::bad_request("bad command"))),
    };
    let request = command(&name, Value::Object(object)).map(|command| AgentRequest {
        version,
        id: id.clone(),
        client,
        command,
    });
    (id, request)
}

/// A JSON number that is a whole number, as `Int` reads one.
fn whole(number: &serde_json::Number) -> Option<i64> {
    number.as_i64().or_else(|| {
        number
            .as_f64()
            .filter(|value| value.fract() == 0.0 && value.abs() < 9.0e15)
            .map(|value| {
                #[allow(clippy::cast_possible_truncation, reason = "a whole number below 2^53")]
                let whole = value as i64;
                whole
            })
    })
}

/// The command `name` with its fields from `fields`, decoded and checked.
#[allow(clippy::too_many_lines, reason = "one arm per command")]
fn command(name: &str, fields: Value) -> Result<AgentCommand, AgentError> {
    fn refuse(problem: Option<String>) -> Result<(), AgentError> {
        problem.map_or(Ok(()), |problem| Err(AgentError::bad_request(problem)))
    }
    Ok(match name {
        "capabilities" => {
            let command: crate::CapabilitiesCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Capabilities(command)
        }
        "show" => {
            let command: crate::ShowCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Show(command)
        }
        "upload" => {
            let command: crate::UploadCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Upload(command)
        }
        "quote" => {
            let command: crate::QuoteCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Quote(command)
        }
        "create_video" => {
            let command: crate::CreateVideoCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::CreateVideo(command)
        }
        "jobs" => {
            let command: crate::JobsCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Jobs(command)
        }
        "job_action" => {
            let command: crate::JobActionCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::JobAction(command)
        }
        "create_image" => {
            let command: crate::CreateImageCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::CreateImage(command)
        }
        "library_search" => {
            let command: crate::LibrarySearchCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::LibrarySearch(command)
        }
        "library_get" => {
            let command: crate::LibraryGetCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::LibraryGet(command)
        }
        "elements" => {
            let command: crate::ElementsCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::Elements(command)
        }
        "create_element" => {
            let command: crate::CreateElementCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::CreateElement(command)
        }
        "library_download" => {
            let command: crate::LibraryDownloadCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::LibraryDownload(command)
        }
        "library_mark" => {
            let command: crate::LibraryMarkCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::LibraryMark(command)
        }
        "enhance" => {
            let command: crate::EnhanceCommand = fields_of(fields)?;
            command.check()?;
            AgentCommand::Enhance(command)
        }
        "delivery_set" => {
            let command: crate::DeliverySetCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::DeliverySet(command)
        }
        "export" => {
            let command: crate::ExportCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::Export(command)
        }
        "frames" => {
            let command: crate::FramesCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::Frames(command)
        }
        "subtitles" => {
            let command: crate::SubtitlesCommand = fields_of(fields)?;
            refuse(command.problem())?;
            AgentCommand::Subtitles(command)
        }
        _ => return Err(AgentError::unknown_command(name)),
    })
}

/// Decodes a command's fields, naming the field a decoding error is about as the Swift app names it: `no <key>` for
/// one missing, `bad <path>` for one of the wrong kind, and the refusal a field's own check made.
pub(crate) fn fields_of<T: DeserializeOwned>(fields: Value) -> Result<T, AgentError> {
    serde_path_to_error::deserialize(fields).map_err(|error| {
        let message = error.inner().to_string();
        if let Some(encoded) = message.strip_prefix(REFUSAL_MARKER)
            && let Ok(refusal) = serde_json::from_str::<AgentError>(encoded)
        {
            return refusal;
        }
        if let Some(rest) = message.strip_prefix("missing field `")
            && let Some((key, _)) = rest.split_once('`')
        {
            return AgentError::bad_request(format!("no {key}"));
        }
        let path: Vec<String> = error
            .path()
            .iter()
            .filter_map(|segment| match segment {
                serde_path_to_error::Segment::Seq { index } => Some(format!("Index {index}")),
                serde_path_to_error::Segment::Map { key } => Some(key.clone()),
                serde_path_to_error::Segment::Enum { variant } => Some(variant.clone()),
                serde_path_to_error::Segment::Unknown => None,
            })
            .collect();
        if path.is_empty() {
            AgentError::bad_request("the fields don't match the command")
        } else {
            AgentError::bad_request(format!("bad {}", path.join(".")))
        }
    })
}

/// Splits a byte stream into lines as it arrives, and hands out the raw bytes of an upload that follow its line.
#[derive(Debug, Default)]
pub struct AgentLineBuffer {
    pending: Vec<u8>,
}

impl AgentLineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
    }

    /// The next complete line without its newline; `None` until one has arrived.
    pub fn next_line(&mut self) -> Option<Vec<u8>> {
        let end = self.pending.iter().position(|byte| *byte == b'\n')?;
        let mut line: Vec<u8> = self.pending.drain(..=end).collect();
        line.pop();
        Some(line)
    }

    /// Up to `count` of the bytes after the last line taken: an upload's payload.
    pub fn take(&mut self, count: usize) -> Vec<u8> {
        let count = count.min(self.pending.len());
        self.pending.drain(..count).collect()
    }

    /// With no line left to take, whether the one arriving is already too long to be a request.
    pub fn is_overlong(&self) -> bool {
        self.pending.len() > MAX_LINE_LENGTH
    }
}
