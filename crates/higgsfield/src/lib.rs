//! Higgsfield clients with no app types.
//!
//! - [`core`]: errors, generation states and the HTTP transport.
//! - [`api`]: the REST API with an API key.
//! - [`mcp`]: OAuth 2.1 with PKCE, the MCP Streamable HTTP client and the account's tools as typed calls.

pub mod api;
pub mod core;
pub mod mcp;
