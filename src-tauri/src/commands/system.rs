//! The tray panel's commands: what it shows and what its rows do.

use jutsu_app::system::tray::{AgentsControl, PanelAction, TrayOpen, TrayPanel};
use tauri::AppHandle;

use super::Environment;
use crate::error::AppError;
use crate::system::{panel, window};

/// What the tray panel shows. `QueueChanged`, `PreferencesChanged` and `ConnectionChanged` say when to ask again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn tray_panel(app: AppHandle, environment: Environment<'_>) -> Result<TrayPanel, AppError> {
    Ok(panel::model(&app, &environment).await)
}

/// Opens the main window on `place` and hides the panel.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn tray_open(app: AppHandle, place: TrayOpen) {
    window::open(&app, &place);
}

/// The tray panel measured its content in logical pixels; the panel takes that height, up to its most.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn tray_fit(app: AppHandle, height: f64) {
    crate::system::panel::fit(&app, height);
}

/// Escape in the tray panel hides it, as a menu bar panel closes.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn tray_close(app: AppHandle) {
    panel::hide(&app);
}

/// Cancel on a running row.
#[tauri::command]
#[specta::specta]
pub(crate) async fn tray_cancel(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    panel::perform(&app, &environment, PanelAction::Cancel(id)).await;
    Ok(())
}

/// Show in File Manager on a recent result.
#[tauri::command]
#[specta::specta]
pub(crate) async fn tray_reveal(
    app: AppHandle,
    environment: Environment<'_>,
    file: String,
) -> Result<(), AppError> {
    panel::perform(&app, &environment, PanelAction::Reveal(file)).await;
    Ok(())
}

/// The header's Pause or Resume of the agents' jobs.
#[tauri::command]
#[specta::specta]
pub(crate) async fn tray_agents(
    app: AppHandle,
    environment: Environment<'_>,
    control: AgentsControl,
) -> Result<(), AppError> {
    panel::perform(&app, &environment, PanelAction::Agents(control)).await;
    Ok(())
}

/// Quit Jutsu Studio: asks first while work runs.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn tray_quit(app: AppHandle) {
    panel::hide(&app);
    window::quit(&app);
}
