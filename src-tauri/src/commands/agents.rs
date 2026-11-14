//! Settings › Agents. What the tab shows comes from `jutsu_app::agents::settings`.

use std::sync::Arc;

use jutsu_app::agents::runtime::AgentRuntime;
use jutsu_app::agents::service::WindowSheet;
use jutsu_app::agents::settings::{self, AgentSettingsView};
use jutsu_core::ProviderKind;
use tauri::State;

use crate::error::AppError;

type Runtime<'a> = State<'a, Arc<AgentRuntime>>;

/// The tab as it is now. `PreferencesChanged` follows every change.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn agents_settings(runtime: Runtime<'_>) -> AgentSettingsView {
    let helper = std::env::current_exe()
        .ok()
        .and_then(|executable| settings::bundled_helper(&executable));
    settings::view(
        runtime.service.preferences(),
        &runtime.status(),
        helper.as_deref(),
    )
}

/// Allow agents.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn agents_set_allowed(runtime: Runtime<'_>, is_allowed: bool) -> Result<(), AppError> {
    Ok(runtime.service.preferences().set_allowed(is_allowed)?)
}

/// Let agents use unlimited generations.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn agents_set_allows_unlimited(runtime: Runtime<'_>, allows: bool) -> Result<(), AppError> {
    Ok(runtime.service.preferences().set_allows_unlimited(allows)?)
}

/// A budget field on commit, `apiKey` (dollars) or `account` (credits); returns what the field shows next.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn agents_commit_budget(
    runtime: Runtime<'_>,
    connection: ProviderKind,
    text: String,
) -> Result<String, AppError> {
    Ok(settings::commit_budget(
        runtime.service.preferences(),
        connection,
        &text,
    )?)
}

/// Send on an agent's image request held for the person (`Review…` › Send). Cancel Request is `images_cancel`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn agents_send_held_image(
    environment: super::Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let id = uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::NotFound("This request is no longer held.".to_owned()))?;
    environment.images.send_held(id);
    Ok(())
}

/// The sheet or dialog the window shows over itself now, `None` once none is open; an agent's `show` doesn't open a
/// sheet over it.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn agents_set_window_sheet(runtime: Runtime<'_>, sheet: Option<WindowSheet>) {
    runtime.service.set_window_sheet(sheet);
}
