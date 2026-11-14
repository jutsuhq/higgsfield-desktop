//! Subtitles made on this computer: the sheet, its run, the editor with Done and the export, and Settings › Subtitles
//! with the speech model's download.

#![allow(clippy::needless_pass_by_value, reason = "commands receive owned arguments")]

use jutsu_app::subtitles::service::{
    SubtitleEdit, SubtitleEditorView, SubtitleRunRow, SubtitleSaveOutcome, SubtitleService,
    SubtitleSettingsView, SubtitleSheetView,
};
use jutsu_core::{MaterialReference, SubtitleEngine, SubtitleFormat, Subtitles};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

fn uuid(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This result is no longer in the library.".to_owned()))
}

fn reference(material_id: &str, version_id: &str) -> Result<MaterialReference, AppError> {
    Ok(MaterialReference::new(uuid(material_id)?, uuid(version_id)?))
}

/// The Subtitles sheet of the video `id`, with the language and engine picked so far.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_sheet(
    environment: Environment<'_>,
    id: String,
    language: Option<String>,
    engine: Option<SubtitleEngine>,
) -> Result<SubtitleSheetView, AppError> {
    Ok(environment.subtitles.sheet(uuid(&id)?, language, engine).await)
}

/// Make: starts the run; `agrees_to_model` once the person ticked the model's download. Answers with the run's ID.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_start(
    environment: Environment<'_>,
    id: String,
    language: String,
    engine: SubtitleEngine,
    agrees_to_model: bool,
) -> Result<String, AppError> {
    environment
        .subtitles
        .start(uuid(&id)?, &language, engine, agrees_to_model)
        .await
        .map_err(|message| AppError::Internal(message.message))
}

/// Stops a run, or drops it while it waits.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_cancel(environment: Environment<'_>, run_id: String) -> Result<(), AppError> {
    environment.subtitles.cancel(uuid(&run_id)?);
    Ok(())
}

/// The runs waiting or running, which the Library's cards show over their frames.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_runs(environment: Environment<'_>) -> Vec<SubtitleRunRow> {
    environment.subtitles.run_rows()
}

/// The failure of the last run or model download, once.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_take_failure(
    environment: Environment<'_>,
) -> Option<jutsu_app::connection::UserMessage> {
    environment.subtitles.take_failure()
}

/// The editor on the video's current version; `None` when it has no subtitles.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_editor(
    environment: Environment<'_>,
    id: String,
) -> Result<Option<SubtitleEditorView>, AppError> {
    Ok(environment.subtitles.editor(uuid(&id)?).await)
}

/// One change of the editor, made on the cues it holds.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_edit(
    material_id: String,
    version_id: String,
    subtitles: Subtitles,
    edit: SubtitleEdit,
) -> Result<SubtitleEditorView, AppError> {
    Ok(SubtitleService::editor_view(
        reference(&material_id, &version_id)?,
        edit.apply(subtitles),
    ))
}

/// Done: saves over what the editor opened with, or with `replace` over what was made meanwhile.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_save(
    environment: Environment<'_>,
    material_id: String,
    version_id: String,
    opened: Subtitles,
    edited: Subtitles,
    replace: bool,
) -> Result<SubtitleSaveOutcome, AppError> {
    let source = reference(&material_id, &version_id)?;
    Ok(environment.subtitles.save(source, &opened, edited, replace).await)
}

/// Export…: the save dialog, named after the video and the language, then the cues as `format` into the file picked,
/// never over one there. Answers the file written; `None` when the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_export(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
    subtitles: Subtitles,
    format: SubtitleFormat,
) -> Result<Option<String>, AppError> {
    let target = environment
        .subtitles
        .target(uuid(&id)?)
        .await
        .ok_or_else(|| AppError::NotFound("This result is no longer in the library.".to_owned()))?;
    let name = SubtitleService::export_name(&target.file, subtitles.language(), format);
    let extension = format.file_extension();
    let dialog = app
        .dialog()
        .file()
        .set_title("Export Subtitles")
        .set_file_name(name)
        .add_filter(extension.to_uppercase(), &[extension]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(path) = picked.and_then(|path| path.into_path().ok()) else {
        return Ok(None);
    };
    SubtitleService::export(&subtitles.cues, format, &path).map_err(AppError::Internal)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Settings › Subtitles.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitle_settings(environment: Environment<'_>) -> SubtitleSettingsView {
    environment.subtitles.settings(SubtitleService::is_available())
}

#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_set_default_engine(environment: Environment<'_>, engine: SubtitleEngine) {
    environment.subtitles.set_default_engine(engine);
}

/// Downloads the speech model, or goes on with a download stopped before.
#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_download_model(environment: Environment<'_>) {
    environment.subtitles.models().download();
}

#[tauri::command]
#[specta::specta]
pub(crate) fn subtitles_cancel_download(environment: Environment<'_>) {
    environment.subtitles.models().cancel();
}

/// Deletes the model, with the folders of earlier revisions, and reads the languages again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn subtitles_remove_model(environment: Environment<'_>) -> Result<(), AppError> {
    environment.subtitles.models().remove();
    environment.subtitles.refresh().await;
    Ok(())
}
