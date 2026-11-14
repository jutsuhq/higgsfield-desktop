//! Enhance on this computer: the sheet of a run, starting and cancelling it, and the runs waiting or running.

use jutsu_app::on_device::service::{OnDeviceRunRow, OnDeviceStart};
use jutsu_app::on_device::sheet::{OnDeviceKind, OnDeviceSheet};
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

fn uuid(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This result is no longer in the library.".to_owned()))
}

/// The sheet of `kind` on the item `id`, with `motion` (a `SmoothMotion` raw value) for Smooth Motion.
#[tauri::command]
#[specta::specta]
pub(crate) async fn on_device_sheet(
    environment: Environment<'_>,
    id: String,
    kind: OnDeviceKind,
    motion: Option<String>,
) -> Result<OnDeviceSheet, AppError> {
    environment
        .on_device
        .sheet(uuid(&id)?, kind, motion.as_deref())
        .await
        .map_err(AppError::NotFound)
}

/// Starts `kind` on the item `id`; `agrees` once the person ticked the download agreement.
#[tauri::command]
#[specta::specta]
pub(crate) async fn on_device_start(
    environment: Environment<'_>,
    id: String,
    kind: OnDeviceKind,
    motion: Option<String>,
    agrees: bool,
) -> Result<OnDeviceStart, AppError> {
    environment
        .on_device
        .start(uuid(&id)?, kind, motion.as_deref(), agrees)
        .await
        .map_err(AppError::NotFound)
}

/// Stops a run, or drops it while it waits.
#[tauri::command]
#[specta::specta]
pub(crate) async fn on_device_cancel(environment: Environment<'_>, run_id: String) -> Result<(), AppError> {
    environment.on_device.cancel(uuid(&run_id)?);
    Ok(())
}

/// The runs waiting or running, the first one running.
#[tauri::command]
#[specta::specta]
pub(crate) async fn on_device_runs(environment: Environment<'_>) -> Result<Vec<OnDeviceRunRow>, AppError> {
    Ok(environment.on_device.run_rows())
}

/// Upscale on This Computer… on a saved image's menu: an image up to 1920×1080, read from its file.
#[tauri::command]
#[specta::specta]
pub(crate) async fn on_device_offers_image_upscale(
    environment: Environment<'_>,
    id: String,
) -> Result<bool, AppError> {
    Ok(environment.on_device.offers_image_upscale(uuid(&id)?).await)
}
