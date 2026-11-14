//! The settings file: read it whole, change any part of it.

use jutsu_app::Preferences;

use super::Environment;
use crate::error::AppError;

/// Every setting the user has changed; absent ones take their defaults.
#[tauri::command]
#[specta::specta]
pub(crate) async fn get_preferences(environment: Environment<'_>) -> Result<Preferences, AppError> {
    Ok(environment.preferences.get())
}

/// Writes the settings `patch` sets and leaves the rest; returns the result. `PreferencesChanged` follows.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_preferences(
    environment: Environment<'_>,
    patch: Preferences,
) -> Result<Preferences, AppError> {
    let preferences = environment.preferences.clone();
    tauri::async_runtime::spawn_blocking(move || preferences.update(patch))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
        .map_err(AppError::from)
}
