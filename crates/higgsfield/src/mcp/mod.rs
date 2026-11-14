//! The Higgsfield MCP server at `mcp.higgsfield.ai`: OAuth sign-in, the Streamable HTTP client and the account's
//! tools as typed calls.

pub mod account;
mod client;
mod error;
mod http;
pub mod json;
pub mod oauth;
mod sse;

pub use client::{McpClient, McpTool, McpToolResult};
pub use error::{JsonRpcError, McpError, OAuthError};
