//! The Settings tabs: General, Output, Storage, Notifications, Shortcuts and Subtitles. What each tab shows comes from
//! `jutsu_app::settings`; a change to the preferences reaches the window as `PreferencesChanged`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use jutsu_app::settings::{
    self, GeneralSettings, KeyStroke, LoginItemStatus, NotificationSettings, OldDrafts, OutputNamePreview,
    OutputSettings, ShortcutAction, ShortcutBinding, ShortcutRecording, ShortcutSettings, ShortcutStore,
    StorageSummary,
};
use jutsu_app::system::updates::{self, UpdateCheck, UpdateSwitch};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use super::Environment;
use crate::error::AppError;

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// Runs a write to the preferences file off the async threads.
async fn write<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, jutsu_app::preferences::PreferencesError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(internal)?
        .map_err(AppError::from)
}

// MARK: General

/// Settings › General.
#[tauri::command]
#[specta::specta]
pub(crate) async fn general_settings(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<GeneralSettings, AppError> {
    let login = match app.autolaunch().is_enabled() {
        Ok(true) => LoginItemStatus::Enabled,
        Ok(false) => LoginItemStatus::Disabled,
        Err(error) => {
            tracing::error!(%error, "can't read the autostart entry");
            LoginItemStatus::Disabled
        }
    };
    Ok(settings::general_settings(
        &environment.preferences.get(),
        login,
        env!("CARGO_PKG_VERSION"),
        &crate::system::updater::updates_state(&app, &environment),
    ))
}

/// Open at login: adds or removes the autostart entry.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_open_at_login(app: AppHandle, is_on: bool) -> Result<(), AppError> {
    let autolaunch = app.autolaunch();
    if is_on {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    }
    .map_err(internal)
}

/// Clear History…: prompts of earlier jobs leave the prompt picker.
#[tauri::command]
#[specta::specta]
pub(crate) async fn clear_prompt_history(environment: Environment<'_>) -> Result<(), AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(internal)?
        .as_secs_f64();
    let preferences = environment.preferences.clone();
    write(move || settings::clear_prompt_history(&preferences, now)).await?;
    Ok(())
}

/// Check Now. A build without an updater says so and fetches nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn check_for_updates(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<UpdateCheck, AppError> {
    Ok(crate::system::updater::check(&app, &environment).await)
}

/// Install Update in the answer to Check Now: downloads, installs and starts the new version.
#[tauri::command]
#[specta::specta]
pub(crate) async fn install_update(app: AppHandle) -> Result<(), AppError> {
    crate::system::updater::install(&app)
        .await
        .map_err(AppError::Internal)
}

/// Check for updates automatically, or Download and install automatically.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_update_switch(
    environment: Environment<'_>,
    which: UpdateSwitch,
    is_on: bool,
) -> Result<(), AppError> {
    let preferences = environment.preferences.clone();
    write(move || updates::set_switch(&preferences, which, is_on)).await
}

// MARK: Output

/// Settings › Output.
#[tauri::command]
#[specta::specta]
pub(crate) async fn output_settings(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<OutputSettings, AppError> {
    let home = app.path().home_dir().unwrap_or_default();
    Ok(settings::output_settings(
        &environment.preferences.get(),
        &environment.layout,
        environment.connection.state().kind(),
        &home,
    ))
}

/// The example name `pattern` gives and its note, in the window's time zone (`utc_offset_minutes` east of UTC).
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn output_name_preview(pattern: String, utc_offset_minutes: i32) -> OutputNamePreview {
    settings::output_name_preview(&pattern, utc_offset_minutes)
}

async fn use_output_folder(environment: Environment<'_>, folder: Option<PathBuf>) -> Result<(), AppError> {
    let (preferences, layout) = (environment.preferences.clone(), environment.layout.clone());
    tauri::async_runtime::spawn_blocking(move || {
        settings::use_output_folder(&preferences, &layout, folder.as_deref())
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    Ok(())
}

/// Change…: picks a folder with the system's folder picker and downloads results there. Nothing changes when the
/// picker is cancelled.
#[tauri::command]
#[specta::specta]
pub(crate) async fn choose_output_folder(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<(), AppError> {
    let current = settings::current_output_folder(&environment.preferences.get(), &environment.layout);
    let dialog = app.dialog().file().set_directory(current);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(internal)?;
    let Some(picked) = picked else {
        return Ok(());
    };
    let folder = picked.into_path().map_err(internal)?;
    use_output_folder(environment, Some(folder)).await
}

/// Use Default: results download to the default folder again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn reset_output_folder(environment: Environment<'_>) -> Result<(), AppError> {
    use_output_folder(environment, None).await
}

/// Opens the output folder in the file manager.
#[tauri::command]
#[specta::specta]
pub(crate) async fn show_output_folder(app: AppHandle, environment: Environment<'_>) -> Result<(), AppError> {
    let folder = settings::current_output_folder(&environment.preferences.get(), &environment.layout);
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(internal)
}

// MARK: Storage

/// Disk use, measured off the async threads.
#[tauri::command]
#[specta::specta]
pub(crate) async fn storage_summary(environment: Environment<'_>) -> Result<StorageSummary, AppError> {
    settings::storage_summary(&environment).await.map_err(internal)
}

/// The library's drafts older than `days` days, 30 unless given.
#[tauri::command]
#[specta::specta]
pub(crate) async fn old_drafts(
    environment: Environment<'_>,
    days: Option<i32>,
) -> Result<OldDrafts, AppError> {
    settings::old_drafts(&environment, days.unwrap_or(settings::DEFAULT_DRAFT_DAYS))
        .await
        .map_err(internal)
}

/// Moves the result files of the drafts the confirmation counted to the trash.
#[tauri::command]
#[specta::specta]
pub(crate) async fn trash_old_drafts(
    environment: Environment<'_>,
    days: i32,
    job_ids: Vec<String>,
) -> Result<(), AppError> {
    let ids = job_ids
        .iter()
        .map(|id| id.parse().map_err(|_| AppError::NotFound(format!("No job {id}"))))
        .collect::<Result<Vec<_>, _>>()?;
    settings::trash_old_drafts(&environment, days, &ids)
        .await
        .map_err(internal)
}

/// Clear Cache: prepared sources, preset previews and downloaded inputs.
#[tauri::command]
#[specta::specta]
pub(crate) async fn clear_cache(environment: Environment<'_>) -> Result<(), AppError> {
    settings::clear_cache(&environment.layout).await.map_err(internal)
}

// MARK: Notifications

/// Settings › Notifications.
#[tauri::command]
#[specta::specta]
pub(crate) async fn notification_settings(
    environment: Environment<'_>,
) -> Result<NotificationSettings, AppError> {
    Ok(settings::notification_settings(&environment.preferences.get()))
}

/// The system settings page for this app's notifications; `None` where there is no portable way to open it.
fn notification_settings_url(app: &AppHandle) -> Option<String> {
    if cfg!(target_os = "windows") {
        Some("ms-settings:notifications".to_owned())
    } else if cfg!(target_os = "macos") {
        Some(format!(
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={}",
            app.config().identifier
        ))
    } else {
        None
    }
}

/// Settings › Notifications' row while the system blocks the app's notifications. The notification plugin answers
/// "granted" on every desktop, so Windows is asked through its registry; Linux has no common switch to read, and there
/// the row never shows.
#[tauri::command]
#[specta::specta]
pub(crate) async fn notifications_blocked(
    app: AppHandle,
) -> Result<Option<jutsu_app::settings::BlockedBySystem>, AppError> {
    let identifier = app.config().identifier.clone();
    let is_blocked = tauri::async_runtime::spawn_blocking(move || system_blocks_notifications(&identifier))
        .await
        .unwrap_or(false);
    Ok(jutsu_app::settings::blocked_by_system(
        is_blocked,
        notification_settings_url(&app).is_some(),
    ))
}

/// Windows' two switches, read with `reg query`: for every app, and for this one under the `AppUserModelID` the
/// notification plugin posts with (the bundle identifier).
#[cfg(target_os = "windows")]
fn system_blocks_notifications(identifier: &str) -> bool {
    use jutsu_app::settings as rules;
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`: no console flashes up.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let query = |key: &str, value: &str| {
        std::process::Command::new("reg")
            .args(["query", key, "/v", value])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    };
    rules::windows_blocks_notifications(
        &query(rules::WINDOWS_ALL_APPS_KEY, rules::WINDOWS_ALL_APPS_VALUE),
        &query(&rules::windows_app_key(identifier), rules::WINDOWS_APP_VALUE),
    )
}

#[cfg(not(target_os = "windows"))]
fn system_blocks_notifications(_identifier: &str) -> bool {
    false
}

/// Open System Settings on that row.
#[tauri::command]
#[specta::specta]
pub(crate) async fn notifications_open_system_settings(app: AppHandle) -> Result<(), AppError> {
    let Some(url) = notification_settings_url(&app) else {
        return Ok(());
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| AppError::Internal(error.to_string()))
}

// MARK: Shortcuts

/// Settings › Shortcuts.
#[tauri::command]
#[specta::specta]
pub(crate) async fn shortcut_settings(environment: Environment<'_>) -> Result<ShortcutSettings, AppError> {
    Ok(ShortcutStore(&environment.preferences).settings())
}

/// The shortcuts the window listens for.
#[tauri::command]
#[specta::specta]
pub(crate) async fn shortcut_bindings(
    environment: Environment<'_>,
) -> Result<Vec<ShortcutBinding>, AppError> {
    Ok(ShortcutStore(&environment.preferences).bindings())
}

/// A key pressed while `action`'s shortcut is recorded: stores it, or says why not.
#[tauri::command]
#[specta::specta]
pub(crate) async fn record_shortcut(
    environment: Environment<'_>,
    action: ShortcutAction,
    stroke: KeyStroke,
) -> Result<ShortcutRecording, AppError> {
    let preferences = environment.preferences.clone();
    write(move || ShortcutStore(&preferences).record(action, &stroke)).await
}

/// Reset: `action` goes back to its default shortcut.
#[tauri::command]
#[specta::specta]
pub(crate) async fn reset_shortcut(
    environment: Environment<'_>,
    action: ShortcutAction,
) -> Result<(), AppError> {
    let preferences = environment.preferences.clone();
    write(move || ShortcutStore(&preferences).save(action, "")).await
}

/// Restore Defaults: every shortcut goes back to its default.
#[tauri::command]
#[specta::specta]
pub(crate) async fn restore_default_shortcuts(environment: Environment<'_>) -> Result<(), AppError> {
    let preferences = environment.preferences.clone();
    write(move || ShortcutStore(&preferences).restore_defaults()).await
}
