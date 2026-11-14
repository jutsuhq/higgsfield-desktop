//! Commands the React app calls through the generated bindings, one module per feature. Each is async, takes the
//! environment from managed state, calls `jutsu-app` and maps the error; no business rule lives here.

pub(crate) mod agents;
pub(crate) mod characters;
pub(crate) mod connection;
pub(crate) mod create;
pub(crate) mod delivery;
pub(crate) mod enhance;
pub(crate) mod images;
pub(crate) mod library;
pub(crate) mod media;
pub(crate) mod on_device;
pub(crate) mod preferences;
pub(crate) mod queue;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod spending;
pub(crate) mod subtitles;
pub(crate) mod system;

use std::sync::Arc;

use jutsu_app::AppEnvironment;

/// The environment as commands receive it.
pub(crate) type Environment<'a> = tauri::State<'a, Arc<AppEnvironment>>;

/// The app's version, from the workspace `Cargo.toml`.
#[tauri::command]
#[specta::specta]
pub(crate) fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
