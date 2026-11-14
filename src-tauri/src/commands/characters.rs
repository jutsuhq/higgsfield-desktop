//! Create Character and References › From Higgsfield › Characters: the logic lives in `jutsu_app::images` and
//! `jutsu_app::create`; the window renders `characters_snapshot` and asks again on `ImagesChanged`.

use jutsu_app::create::IMAGE_EXTENSIONS;
use jutsu_app::images::service::character_sheet::{CharacterEdit, CharacterSnapshot};
use jutsu_app::images::service::{ImageGenerateOutcome, ReferencePicksView};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

/// Create Character as it stands.
#[tauri::command]
#[specta::specta]
pub(crate) async fn characters_snapshot(environment: Environment<'_>) -> Result<CharacterSnapshot, AppError> {
    Ok(environment.images.character_snapshot())
}

/// One change to the form; answers with the sheet after it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn characters_edit(
    environment: Environment<'_>,
    edit: CharacterEdit,
) -> Result<CharacterSnapshot, AppError> {
    Ok(environment.images.edit_character(edit))
}

/// Choose… on the Face row (one image) or the Style row (several).
#[tauri::command]
#[specta::specta]
pub(crate) async fn characters_choose_images(
    app: AppHandle,
    environment: Environment<'_>,
    styles: bool,
) -> Result<CharacterSnapshot, AppError> {
    let dialog = app
        .dialog()
        .file()
        .set_title(if styles {
            "Choose Style Images"
        } else {
            "Choose a Face"
        })
        .add_filter("Image", &IMAGE_EXTENSIONS);
    let picked = if styles {
        tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .unwrap_or_default()
    } else {
        tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .into_iter()
            .collect()
    };
    let mut paths = Vec::with_capacity(picked.len());
    for file in picked {
        let path = file
            .into_path()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        paths.push(path.to_string_lossy().into_owned());
    }
    if paths.is_empty() {
        return Ok(environment.images.character_snapshot());
    }
    Ok(environment.images.edit_character(if styles {
        CharacterEdit::AddStyles { paths }
    } else {
        CharacterEdit::DropFace { paths }
    }))
}

/// The next page of presets.
#[tauri::command]
#[specta::specta]
pub(crate) async fn characters_load_more_presets(environment: Environment<'_>) -> Result<(), AppError> {
    environment.images.load_more_character_presets().await;
    Ok(())
}

/// Generate. Over today's cap it returns the question; ask, then call again `approved`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn characters_generate(
    environment: Environment<'_>,
    approved: bool,
) -> Result<ImageGenerateOutcome, AppError> {
    Ok(environment.images.generate_characters(approved).await)
}

/// Picks a sheet of References › From Higgsfield › Characters; answers why it wasn't added.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_character(
    environment: Environment<'_>,
    id: String,
) -> Result<Option<String>, AppError> {
    Ok(environment.create.add_character(&id).await?)
}

/// Generate…'s picks for References; the results come from `images_snapshot`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_reference_picks(
    environment: Environment<'_>,
) -> Result<ReferencePicksView, AppError> {
    Ok(environment
        .images
        .reference_picks(environment.create.reference_room()))
}

/// A click on a result of Generate…: picks it, or unpicks it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_toggle_reference_pick(
    environment: Environment<'_>,
    id: String,
) -> Result<ReferencePicksView, AppError> {
    let room = environment.create.reference_room();
    Ok(environment.images.toggle_reference_pick(image_id(&id)?, room))
}

/// Generate… closed: the picks go, the images stay.
#[tauri::command]
#[specta::specta]
pub(crate) async fn images_clear_reference_picks(environment: Environment<'_>) -> Result<(), AppError> {
    environment.images.clear_reference_picks();
    Ok(())
}

/// Add N to References: downloads each pick in the order picked and adds it; answers why one wasn't added, which
/// stops there and keeps the rest picked.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_generated(
    environment: Environment<'_>,
    id: String,
) -> Result<Option<String>, AppError> {
    let id = image_id(&id)?;
    let Some(image) = environment
        .images
        .picked_images()
        .into_iter()
        .find(|image| image.id == id)
    else {
        return Ok(None);
    };
    let message = environment.create.add_generated(&image).await?;
    if message.is_none() {
        environment.images.mark_added(id);
    }
    Ok(message)
}

fn image_id(text: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(text)
        .map_err(|_| AppError::NotFound("This image is no longer in the results.".to_owned()))
}
