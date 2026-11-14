//! Both Higgsfield connections (API key and account) as `GenerationProvider`s over the `higgsfield` crate.
//!
//! - [`HiggsfieldApiProvider`]: the REST API with a key, paying in USD; also an `ImageProvider` and a
//!   `ModelConstraintsSource`.
//! - [`HiggsfieldMcpProvider`]: a Higgsfield account over MCP, paying in credits; also every optional service the
//!   account offers (history, likes, credit history, uploads, Motion Library, constraints, images, character
//!   sheets, elements, voices and virality).
//!
//! Each module is the port of the Swift file of the same name in `HiggsfieldProviders`.
#![allow(
    clippy::doc_markdown,
    reason = "the docs keep the Swift original's wording, tool and model names included"
)]
#![allow(
    clippy::must_use_candidate,
    reason = "most functions are pure mappings; marking each adds noise"
)]

mod account_characters;
mod account_charges;
mod account_elements;
mod account_images;
mod account_mapping;
mod account_model;
mod account_virality;
mod account_voices;
mod api_model;
mod api_provider;
mod genjutsu_mapping;
mod history_mapping;
mod mapping;
mod mcp_provider;
mod quote_cache;

pub use api_provider::HiggsfieldApiProvider;
pub use mapping::{
    account_sign_in_error, image_status, oauth_tokens, provider_error, rejection, remote_status,
    sign_in_error, token_set,
};
pub use mcp_provider::{ConstraintsSource, HiggsfieldMcpProvider, UnlimitedOptIn};

#[cfg(test)]
mod tests;
