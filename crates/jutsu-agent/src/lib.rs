//! The agent socket: wire types, framing, listener, client and inbox. A named pipe on Windows, a Unix socket in the
//! runtime folder on Linux and macOS. The wire is the macOS app's, field for field (see `ARCHITECTURE.md`, Agents).
#![allow(clippy::must_use_candidate, reason = "plain accessors and constructors")]
#![allow(clippy::missing_errors_doc, reason = "each error type documents its cases")]

pub mod capabilities;
pub mod client;
pub mod commands;
pub mod delivery;
pub mod elements;
pub mod enhance;
pub mod error;
pub mod fingerprint;
pub mod inbox;
pub mod job;
pub mod job_commands;
pub mod json;
pub mod library;
pub mod names;
pub mod operations;
pub mod protocol;
pub mod quote;
pub mod server;
pub mod socket;
pub mod subtitles;
pub mod values;
pub mod video;
pub mod wire;

pub use capabilities::*;
pub use client::AgentClient;
pub use commands::*;
pub use delivery::*;
pub use elements::*;
pub use enhance::*;
pub use error::{AgentError, Error, ErrorCode};
pub use inbox::AgentInbox;
pub use job::*;
pub use job_commands::*;
pub use library::*;
pub use operations::*;
pub use protocol::*;
pub use quote::*;
pub use server::{AgentHandler, AgentServer};
pub use socket::AgentEndpoint;
pub use subtitles::*;
pub use values::{Timestamp, Uuid};
pub use video::*;
pub use wire::AgentLineBuffer;

#[cfg(test)]
mod tests;
