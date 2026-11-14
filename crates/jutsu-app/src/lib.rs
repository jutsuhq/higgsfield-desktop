//! UI logic without UI: the composition root, preferences, the connection store and what the UI shows of it, the
//! `keyring` implementation of the credential vault, the active provider, the loopback media server and log
//! redaction. Nothing here knows Tauri; `src-tauri` adapts it to commands and events.

#![allow(clippy::return_self_not_must_use, reason = "as `must_use_candidate`")]
#![allow(
    clippy::must_use_candidate,
    reason = "most of the crate is pure queries; marking each adds noise"
)]

pub mod agents;
pub mod connection;
pub mod create;
pub mod environment;
pub mod images;
pub mod library;
pub mod logging;
pub mod media_server;
pub mod on_device;
pub mod preferences;
pub mod prompts;
pub mod provider;
pub mod queue;
pub mod settings;
pub mod subtitles;
pub mod system;
pub mod templates;

#[cfg(test)]
mod testing;

pub use environment::{AppDirectories, AppEnvironment, EnvironmentError, PlatformServices};
pub use media_server::{MediaServer, MediaServerError};
pub use preferences::{Preferences, PreferencesStore};
pub use provider::{ActiveProvider, HiggsfieldProviderFactory, ProviderFactory, ProviderHandle};
