//! The macOS app's `JutsuAgentTests`, case for case. The spelling and request-ID cases of `WireTests` are in `names`,
//! the inbox sweep in `inbox`, the exports in `operations`.

mod on_device;
mod server;
mod subtitles;
mod video_commands;
mod wire;

use crate::{AgentCommand, AgentError, AgentRequest, wire as agent_wire};

/// `command` sent as a line and read back.
fn round_trip(command: &AgentCommand) -> AgentCommand {
    let mut request = AgentRequest::new(command.clone());
    request.id = Some("1".to_owned());
    let line = agent_wire::line(&request).expect("line");
    agent_wire::request(&line[..line.len() - 1])
        .1
        .expect("request")
        .command
}

/// The error `json` is refused with, if it is.
fn error(json: &str) -> Option<AgentError> {
    agent_wire::request(json.as_bytes()).1.err()
}

const FINGERPRINT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FINGERPRINT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
