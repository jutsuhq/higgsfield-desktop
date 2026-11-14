//! Delivery sets: the sheet's list, a set's detail and its edits, New Delivery Set from a Library selection, the
//! next round, delete, the frame sheet, and exporting a set's files into a folder with each file's state sent as it
//! changes.

use std::path::Path;
use std::sync::Arc;

use jutsu_app::MediaServer;
use jutsu_app::library::LibraryError;
use jutsu_app::library::delivery::{
    DeliverySetChange, DeliverySetRow, DeliverySetView, DeliveryTaskState, FOLDER_MESSAGE, FrameRect,
};
use jutsu_app::library::service::{DeliveryExportScope, DeliveryFrameView, DeliveryRunSummary};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

fn set_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("Can't open this delivery set".to_owned()))
}

fn library_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Catalog(_) => AppError::Internal(error.to_string()),
        LibraryError::NotFound(message) => AppError::NotFound(message),
    }
}

/// One file's state in a run, by `DeliveryTask::id` ("<version ID>/<output ID>").
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeliveryTaskUpdate {
    task_id: String,
    state: DeliveryTaskState,
}

/// The sheet's list, newest first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_sets(environment: Environment<'_>) -> Result<Vec<DeliverySetRow>, AppError> {
    environment.library.delivery_sets().await.map_err(library_error)
}

/// One set's detail.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_set(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<DeliverySetView, AppError> {
    let url_for = |path: &Path| server.url_for(path).ok();
    environment
        .library
        .delivery_set(set_id(&id)?, &url_for)
        .await
        .map_err(library_error)
}

/// New Delivery Set of the selected Library materials; answers its ID.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_new(environment: Environment<'_>, ids: Vec<String>) -> Result<String, AppError> {
    let ids: Vec<Uuid> = ids.iter().filter_map(|id| Uuid::parse_str(id).ok()).collect();
    environment
        .library
        .new_delivery_set(&ids)
        .await
        .map(|id| id.to_string())
        .map_err(AppError::Internal)
}

/// Applies one edit to a set and saves it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_change(
    environment: Environment<'_>,
    id: String,
    change: DeliverySetChange,
) -> Result<(), AppError> {
    environment
        .library
        .change_delivery_set(set_id(&id)?, change)
        .await
        .map_err(AppError::Internal)
}

/// New Set from This One; answers the new set's ID.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_next_round(
    environment: Environment<'_>,
    id: String,
) -> Result<String, AppError> {
    environment
        .library
        .next_delivery_round(set_id(&id)?)
        .await
        .map(|id| id.to_string())
        .map_err(AppError::Internal)
}

/// Delete Set: the set only; exported files and the library's versions stay.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_delete(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    environment
        .library
        .delete_delivery_set(set_id(&id)?)
        .await
        .map_err(AppError::Internal)
}

/// The frame sheet of one item for one aspect.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_frame(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
    version_id: String,
    aspect: String,
) -> Result<DeliveryFrameView, AppError> {
    let url_for = |path: &Path| server.url_for(path).ok();
    let version = Uuid::parse_str(&version_id)
        .map_err(|_| AppError::NotFound("This item is no longer in the set.".to_owned()))?;
    environment
        .library
        .delivery_frame(set_id(&id)?, version, &aspect, &url_for)
        .await
        .map_err(library_error)
}

/// Export…, Export N Pending or Retry N Failed. Export… always asks for a folder; the others write to the set's own
/// folder while it exists. Each file's state goes out on `on_task`. `None` when no folder was picked.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_export(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
    scope: DeliveryExportScope,
    on_task: Channel<DeliveryTaskUpdate>,
) -> Result<Option<DeliveryRunSummary>, AppError> {
    let id = set_id(&id)?;
    let own = if scope == DeliveryExportScope::All {
        None
    } else {
        environment.library.delivery_folder(id).await
    };
    let folder = if let Some(folder) = own {
        folder
    } else {
        let dialog = app.dialog().file().set_title(FOLDER_MESSAGE);
        let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let Some(folder) = picked.and_then(|picked| picked.into_path().ok()) else {
            return Ok(None);
        };
        folder
    };
    let report = Arc::new(move |task: &str, state: DeliveryTaskState| {
        let _ = on_task.send(DeliveryTaskUpdate {
            task_id: task.to_owned(),
            state,
        });
    });
    environment
        .library
        .export_delivery_set(id, scope, &folder, report)
        .await
        .map(Some)
        .map_err(AppError::Internal)
}

/// Cancel Export.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_cancel(environment: Environment<'_>) -> Result<(), AppError> {
    environment.library.cancel_delivery_export();
    Ok(())
}

/// Show in file manager: the folder the set was exported to.
#[tauri::command]
#[specta::specta]
pub(crate) async fn delivery_show_folder(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let Some(folder) = environment.library.delivery_folder(set_id(&id)?).await else {
        return Ok(());
    };
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|error| AppError::Internal(error.to_string()))
}

/// The frame sheet's crop in fractions of the picture, for the frame and picture shape on screen.
#[tauri::command]
#[specta::specta]
#[allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]
pub(crate) fn delivery_crop_rect(
    frame: jutsu_app::library::delivery::CropFrame,
    aspect: String,
    picture_aspect: f64,
) -> Option<FrameRect> {
    jutsu_app::library::delivery::crop_rect(frame, &aspect, picture_aspect)
}
