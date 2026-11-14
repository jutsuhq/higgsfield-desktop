//! Settings › Spending: the summary of a period, the CSV export, the cap and low-credit fields, and the account's
//! credit history. What each shows comes from `jutsu_app::settings::spending`.

use jutsu_app::settings::spending::{CreditHistoryView, SpendingPeriod, SpendingView};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;

use super::Environment;
use crate::error::AppError;

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// The Spending tab for `period`; `None` while nothing is connected.
#[tauri::command]
#[specta::specta]
pub(crate) async fn spending_view(
    environment: Environment<'_>,
    period: SpendingPeriod,
) -> Result<Option<SpendingView>, AppError> {
    environment
        .library
        .spending(period)
        .await
        .map_err(AppError::Internal)
}

/// The Daily cap toggle of the connection's unit. `PreferencesChanged` follows.
#[tauri::command]
#[specta::specta]
pub(crate) async fn spending_set_cap_enabled(
    environment: Environment<'_>,
    is_on: bool,
) -> Result<(), AppError> {
    let library = environment.library.clone();
    tauri::async_runtime::spawn_blocking(move || library.set_spend_cap_enabled(is_on))
        .await
        .map_err(internal)?
        .map_err(AppError::from)
}

/// The Amount field on commit; returns what the field shows next.
#[tauri::command]
#[specta::specta]
pub(crate) async fn spending_commit_cap(
    environment: Environment<'_>,
    text: String,
) -> Result<String, AppError> {
    let library = environment.library.clone();
    tauri::async_runtime::spawn_blocking(move || library.commit_spend_cap(&text))
        .await
        .map_err(internal)?
        .map_err(AppError::from)
}

/// The Warn below field on commit; returns what the field shows next.
#[tauri::command]
#[specta::specta]
pub(crate) async fn spending_commit_low_credits(
    environment: Environment<'_>,
    text: String,
) -> Result<String, AppError> {
    let library = environment.library.clone();
    tauri::async_runtime::spawn_blocking(move || library.commit_low_credits(&text))
        .await
        .map_err(internal)?
        .map_err(AppError::from)
}

/// Export CSV…: asks where to save, then writes every finished job and image charge. `false` when cancelled; the
/// error is the "Can't export" alert's message.
#[tauri::command]
#[specta::specta]
pub(crate) async fn spending_export_csv(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<bool, AppError> {
    let (csv, name) = environment
        .library
        .spending_csv()
        .await
        .map_err(AppError::Internal)?;
    let dialog = app
        .dialog()
        .file()
        .set_file_name(format!("{name}.csv"))
        .add_filter("CSV", &["csv"]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(internal)?;
    let Some(path) = picked else {
        return Ok(false);
    };
    let path = path.into_path().map_err(internal)?;
    tokio::fs::write(&path, csv).await.map_err(internal)?;
    Ok(true)
}

/// Credit history: the first page loads on first view. `None` for an API key.
#[tauri::command]
#[specta::specta]
pub(crate) async fn credit_history(
    environment: Environment<'_>,
) -> Result<Option<CreditHistoryView>, AppError> {
    Ok(environment.library.credit_history().await)
}

/// Show More.
#[tauri::command]
#[specta::specta]
pub(crate) async fn credit_history_more(
    environment: Environment<'_>,
) -> Result<Option<CreditHistoryView>, AppError> {
    Ok(environment.library.credit_history_more().await)
}

/// Try Again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn credit_history_retry(
    environment: Environment<'_>,
) -> Result<Option<CreditHistoryView>, AppError> {
    Ok(environment.library.credit_history_retry().await)
}
