//! The Higgsfield connection: its state, connecting with a key or an account, signing out.

use jutsu_app::connection::{Activity, ApiKeyForm, ConnectionDetails, ConnectionSnapshot, higgsfield_urls};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use super::Environment;
use crate::error::AppError;

/// The connection as the window shows it; `ConnectionChanged` carries every later change.
#[tauri::command]
#[specta::specta]
pub(crate) async fn connection_status(environment: Environment<'_>) -> Result<ConnectionSnapshot, AppError> {
    Ok(environment.connection_snapshot())
}

/// The Settings › Connection card. Asks the provider what it runs, so it can take a moment.
#[tauri::command]
#[specta::specta]
pub(crate) async fn connection_details(environment: Environment<'_>) -> Result<ConnectionDetails, AppError> {
    Ok(environment.connection_details().await)
}

/// Checks the key with Higgsfield and keeps it when it works. Returns once the check is over; the outcome is in
/// the next snapshot (connected, or an alert).
#[tauri::command]
#[specta::specta]
pub(crate) async fn connect_api_key(
    environment: Environment<'_>,
    key_id: String,
    secret: String,
) -> Result<(), AppError> {
    environment
        .connection
        .connect_api_key(&ApiKeyForm::new(key_id, secret))
        .await;
    Ok(())
}

/// Opens Higgsfield's sign-in page in the browser and waits for the redirect in the background.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_sign_in(environment: Environment<'_>) -> Result<(), AppError> {
    drop(environment.connection.start_sign_in());
    Ok(())
}

/// Stops waiting for the browser.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel_sign_in(environment: Environment<'_>) -> Result<(), AppError> {
    environment.connection.cancel_sign_in();
    Ok(())
}

/// Opens the sign-in page again, for the sheet's Open Again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn open_sign_in_page(app: AppHandle, environment: Environment<'_>) -> Result<(), AppError> {
    let Activity::SigningIn(Some(url)) = environment.connection.activity() else {
        return Err(AppError::NotFound(
            "No sign-in is waiting for the browser.".to_owned(),
        ));
    };
    open(&app, url.as_str())
}

/// Opens the Higgsfield console, where API keys are made.
#[tauri::command]
#[specta::specta]
pub(crate) async fn open_higgsfield_console(app: AppHandle) -> Result<(), AppError> {
    open(&app, higgsfield_urls::CONSOLE.as_str())
}

/// Deletes the stored credential and disconnects; an account's tokens are revoked in the background.
#[tauri::command]
#[specta::specta]
pub(crate) async fn sign_out(environment: Environment<'_>) -> Result<(), AppError> {
    environment.connection.sign_out().await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn dismiss_connection_alert(environment: Environment<'_>) -> Result<(), AppError> {
    environment.connection.dismiss_alert();
    Ok(())
}

fn open(app: &AppHandle, url: &str) -> Result<(), AppError> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| AppError::Internal(format!("Can't open the browser: {error}")))
}
