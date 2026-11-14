//! The MCP server the `jutsu` helper runs: JSON-RPC over stdio and its tools, which call the app through the agent
//! socket. Every tool is one `AgentCommand`; the helper checks its arguments, hands the agent's files to the app's
//! inbox and passes the command on.
#![allow(clippy::must_use_candidate, reason = "plain accessors and constructors")]
#![allow(clippy::missing_errors_doc, reason = "each error type documents its cases")]

pub mod app_connection;
mod arguments;
pub mod connection;
mod definitions;
mod descriptions;
pub mod inbox;
mod library_tools;
mod schemas;
pub mod server;
pub mod tools;

pub use app_connection::{App, AppConnection, AppLauncher, Location, RunningApp};
pub use connection::{McpConnection, Write, serve};
pub use inbox::Inbox;
pub use server::McpServer;
pub use tools::Tool;

/// The crate's one error: reading the client's lines failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("couldn't read the client's messages: {0}")]
    Read(#[from] std::io::Error),
}

#[cfg(test)]
mod tests;
