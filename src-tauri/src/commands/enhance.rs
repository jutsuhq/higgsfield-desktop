//! Enhance and Export for Editing on a result's page: the sheet of one operation with its price, queueing it and
//! Change Voice's voices; the export menu, the export into a picked folder and its cancel.

use jutsu_app::create::SubmitOutcome;
use jutsu_app::library::editing_export::{EditingExportMenu, ExportPreset, FOLDER_MESSAGE};
use jutsu_app::library::enhance::PostActionKind;
use jutsu_app::library::enhance::{EnhanceChoice, EnhanceSheet, VoiceRow};
use std::sync::Arc;
use tauri::AppHandle;
use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;
use jutsu_app::library::LibraryError;

fn material_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This result is no longer in the library.".to_owned()))
}

fn library_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Catalog(_) => AppError::Internal(error.to_string()),
        LibraryError::NotFound(message) => AppError::NotFound(message),
    }
}

/// The options a menu item opens its sheet with.
#[tauri::command]
#[specta::specta]
pub(crate) fn enhance_choice(kind: PostActionKind) -> EnhanceChoice {
    EnhanceChoice::new(kind)
}

/// The sheet of `choice` on the result `id`, priced by Higgsfield.
#[tauri::command]
#[specta::specta]
pub(crate) async fn enhance_sheet(
    environment: Environment<'_>,
    id: String,
    choice: EnhanceChoice,
) -> Result<EnhanceSheet, AppError> {
    environment
        .library
        .enhance_sheet(material_id(&id)?, choice)
        .await
        .map_err(library_error)
}

/// Queues `choice` on the result `id`; `approved` once the person agreed to go over the cap.
#[tauri::command]
#[specta::specta]
pub(crate) async fn enhance_queue(
    environment: Environment<'_>,
    id: String,
    choice: EnhanceChoice,
    approved: bool,
) -> Result<SubmitOutcome, AppError> {
    Ok(environment
        .library
        .enhance(material_id(&id)?, choice, approved)
        .await)
}

/// The voices Change Voice offers; the error's message is the notice shown instead.
#[tauri::command]
#[specta::specta]
pub(crate) async fn enhance_voices(environment: Environment<'_>) -> Result<Vec<VoiceRow>, AppError> {
    environment.library.voices().await.map_err(AppError::NotFound)
}

/// The Export for Editing menu of the result `id`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn editing_export_menu(
    environment: Environment<'_>,
    id: String,
) -> Result<EditingExportMenu, AppError> {
    environment
        .library
        .editing_export_menu(material_id(&id)?)
        .await
        .map_err(library_error)
}

/// Picks a folder and exports the result `id` there with `preset`, sending the fraction done on `on_progress`, then
/// shows the video in the file manager. Answers whether a file was written.
#[tauri::command]
#[specta::specta]
pub(crate) async fn editing_export(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
    preset: ExportPreset,
    on_progress: Channel<f64>,
) -> Result<bool, AppError> {
    let id = material_id(&id)?;
    let dialog = app.dialog().file().set_title(FOLDER_MESSAGE);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(folder) = picked.and_then(|picked| picked.into_path().ok()) else {
        return Ok(false);
    };
    let progress = Arc::new(move |fraction: f64| {
        let _ = on_progress.send(fraction);
    });
    let Some(video) = environment
        .library
        .export_for_editing(id, preset, &folder, progress)
        .await
    else {
        return Ok(false);
    };
    app.opener()
        .reveal_item_in_dir(video)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(true)
}

/// Cancel Export.
#[tauri::command]
#[specta::specta]
pub(crate) async fn editing_export_cancel(environment: Environment<'_>) -> Result<(), AppError> {
    environment.library.cancel_editing_export();
    Ok(())
}
