//! The Image side of Create, the Templates menu and the prompt history: the logic lives in `jutsu_app::images`,
//! `jutsu_app::templates` and `jutsu_app::prompts`; the window renders `images_snapshot` and asks again on
//! `ImagesChanged`.

use jutsu_app::create::{IMAGE_EXTENSIONS, Notice};
use jutsu_app::images::service::{CreateSide, ImageEdit, ImageGenerateOutcome, ImageSnapshot};
use jutsu_app::prompts::PromptHistoryView;
use jutsu_app::templates::{TemplateError, TemplateRow, not_saved};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;
use time::OffsetDateTime;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

impl From<TemplateError> for AppError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::NotFound(_) => Self::NotFound(error.to_string()),
            TemplateError::UnreadableImage | TemplateError::Store(_) => Self::Internal(error.to_string()),
        }
    }
}

fn id(text: &str, missing: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(text).map_err(|_| AppError::NotFound(missing.to_owned()))
}

/// Everything the Image side shows now. `ImagesChanged` says when to ask again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_snapshot(environment: Environment<'_>) -> Result<ImageSnapshot, AppError> {
    Ok(environment.images.snapshot())
}

/// One change to the image form; answers with the side after it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_edit(
    environment: Environment<'_>,
    edit: ImageEdit,
) -> Result<ImageSnapshot, AppError> {
    Ok(environment.images.edit(edit))
}

/// The Video / Image switch.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_set_side(environment: Environment<'_>, side: CreateSide) -> Result<(), AppError> {
    environment.images.set_side(side);
    Ok(())
}

/// Choose…: picks the image to edit.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_choose_input(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<ImageSnapshot, AppError> {
    let dialog = app
        .dialog()
        .file()
        .set_title("Choose an Image to Edit")
        .add_filter("Image", &IMAGE_EXTENSIONS);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let path = picked
        .map(|file| {
            file.into_path()
                .map_err(|error| AppError::Internal(error.to_string()))
        })
        .transpose()?;
    Ok(match path {
        Some(path) => environment.images.edit(ImageEdit::SetInput {
            path: Some(path.to_string_lossy().into_owned()),
        }),
        None => environment.images.snapshot(),
    })
}

/// Generate. Over today's cap, or with no price under one, it returns the question; ask, then call again
/// `approved`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_generate(
    environment: Environment<'_>,
    approved: bool,
) -> Result<ImageGenerateOutcome, AppError> {
    Ok(environment.images.generate(approved).await)
}

/// Cancel on a running request.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_cancel(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    environment.images.cancel(id_of_run(&id)?);
    Ok(())
}

fn id_of_run(text: &str) -> Result<Uuid, AppError> {
    id(text, "This request is no longer running.")
}

/// Shows image `id` large; `None` follows the newest.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_show(environment: Environment<'_>, id: Option<String>) -> Result<(), AppError> {
    let image = id
        .map(|text| self::id(&text, "This image is no longer in the results."))
        .transpose()?;
    environment.images.show(image);
    Ok(())
}

/// Moves image `id` to the Trash and takes it out of the results.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_delete(
    environment: Environment<'_>,
    id: String,
) -> Result<Option<Notice>, AppError> {
    let image = self::id(&id, "This image is no longer in the results.")?;
    Ok(environment.images.delete(image).await.err())
}

/// Clears the results; the saved images stay in the Library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_clear(environment: Environment<'_>) -> Result<(), AppError> {
    environment.images.clear();
    Ok(())
}

/// Try Again on the records that couldn't be saved.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_retry_saving(environment: Environment<'_>) -> Result<(), AppError> {
    environment.images.retry_saving_records().await;
    Ok(())
}

/// OK on an image that couldn't be saved to the Library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_dismiss_save_failure(environment: Environment<'_>) -> Result<(), AppError> {
    environment.images.dismiss_save_failure();
    Ok(())
}

/// The Templates menu, the templates of `side` first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_list(
    environment: Environment<'_>,
    side: CreateSide,
) -> Result<Vec<TemplateRow>, AppError> {
    Ok(environment.templates.list(side).await?)
}

/// What Save as Template's name field starts with.
#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_suggested_name(
    environment: Environment<'_>,
    side: CreateSide,
) -> Result<String, AppError> {
    Ok(environment.templates.suggested_name(side))
}

/// Saves the form of `side` as a template; `Err` holds the alert. Answers with the name it got.
#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_save(
    environment: Environment<'_>,
    side: CreateSide,
    name: String,
) -> Result<TemplateSaveOutcome, AppError> {
    Ok(match environment.templates.save(side, &name).await {
        Ok(name) => TemplateSaveOutcome::Saved { name },
        Err(error) => TemplateSaveOutcome::Failed {
            notice: not_saved(&error),
        },
    })
}

/// What Save as Template did.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TemplateSaveOutcome {
    Saved { name: String },
    Failed { notice: Notice },
}

/// Renames template `id`; answers with the name it got.
#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_rename(
    environment: Environment<'_>,
    id: String,
    name: String,
) -> Result<String, AppError> {
    Ok(environment
        .templates
        .rename(self::id(&id, "This template no longer exists.")?, &name)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_delete(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    Ok(environment
        .templates
        .delete(self::id(&id, "This template no longer exists.")?)
        .await?)
}

/// Fills the Create form of the template's kind from it; the notice says when its model isn't offered.
#[tauri::command]
#[specta::specta]
pub(crate) async fn templates_apply(
    environment: Environment<'_>,
    id: String,
) -> Result<Option<Notice>, AppError> {
    Ok(environment
        .templates
        .apply(self::id(&id, "This template no longer exists.")?)
        .await?)
}

/// The Prompt History popover, filtered by `query`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prompts_history(
    environment: Environment<'_>,
    query: String,
) -> Result<PromptHistoryView, AppError> {
    environment
        .prompts
        .history(&query, OffsetDateTime::now_utc())
        .await
        .map_err(AppError::Internal)
}

/// Star or Unstar on a row.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prompts_set_starred(
    environment: Environment<'_>,
    text: String,
    starred: bool,
) -> Result<(), AppError> {
    environment
        .prompts
        .set_starred(&text, starred)
        .await
        .map_err(AppError::Internal)
}
