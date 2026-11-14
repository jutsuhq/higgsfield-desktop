//! URLs `<video>` and `<img>` load files from, through the loopback media server.

use std::path::PathBuf;

use jutsu_app::MediaServer;
use tauri::State;

use crate::error::AppError;

/// The media server URL of the file at `path`. Refuses files outside the output and cache folders.
#[tauri::command]
#[specta::specta]
pub(crate) async fn media_url(server: State<'_, MediaServer>, path: String) -> Result<String, AppError> {
    Ok(server.url_for(&PathBuf::from(path))?)
}
