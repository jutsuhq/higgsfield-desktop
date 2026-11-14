//! Create, the video side: the form lives in `jutsu_app::create`; the window renders `create_snapshot`, sends each
//! change as `create_edit` and asks again on `CreateChanged`.

use std::path::PathBuf;

use jutsu_app::connection::UserMessage;
use jutsu_app::create::account_references::{ElementOpening, HiggsfieldReferencesView, HiggsfieldSource};
use jutsu_app::create::links::{self, LinkEntryView, LinkOutcome};
use jutsu_app::create::motion::MotionLibraryView;
use jutsu_app::create::{
    AUDIO_EXTENSIONS, AddOutcome, CreateEdit, CreateError, CreateSnapshot, FrameSlot, IMAGE_EXTENSIONS,
    LoadOutcome, Notice, RoleChangeOutcome, SubmitOutcome, VIDEO_EXTENSIONS,
};

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

impl From<CreateError> for AppError {
    fn from(error: CreateError) -> Self {
        match error {
            CreateError::NotFound(_) => Self::NotFound(error.to_string()),
            CreateError::Media(_) | CreateError::Invalid(_) | CreateError::Io(_) => {
                Self::Internal(error.to_string())
            }
        }
    }
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

fn job_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This job is no longer in the queue.".to_owned()))
}

/// The files picked in an open panel; empty when it was cancelled.
async fn pick(
    app: &AppHandle,
    title: &str,
    kind: &str,
    extensions: &[&str],
    many: bool,
) -> Result<Vec<PathBuf>, AppError> {
    let dialog = app.dialog().file().set_title(title).add_filter(kind, extensions);
    let picked = if many {
        tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
            .await
            .map_err(internal)?
            .unwrap_or_default()
    } else {
        tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
            .await
            .map_err(internal)?
            .into_iter()
            .collect()
    };
    picked
        .into_iter()
        .map(|file| file.into_path().map_err(internal))
        .collect()
}

/// Everything Create shows now. `CreateChanged` says when to ask again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_snapshot(environment: Environment<'_>) -> Result<CreateSnapshot, AppError> {
    Ok(environment.create.snapshot().await)
}

/// One change to the form; answers with the form after it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_edit(
    environment: Environment<'_>,
    edit: CreateEdit,
) -> Result<CreateSnapshot, AppError> {
    Ok(environment.create.edit(edit).await?)
}

/// A change of tab, task or model: the form after it, what it did to the images' roles, and whether Undo applies.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_change_roles(
    environment: Environment<'_>,
    edit: CreateEdit,
) -> Result<RoleChangeOutcome, AppError> {
    Ok(environment.create.change_roles(edit).await?)
}

/// Open…: picks a video and opens it as the source.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_choose_source(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<(), AppError> {
    let picked = pick(&app, "Open a Video", "Video", &VIDEO_EXTENSIONS, false).await?;
    if let Some(path) = picked.first() {
        environment.create.open_source(path).await?;
    }
    Ok(())
}

/// Opens the video at `path` as the source.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_open_source(environment: Environment<'_>, path: String) -> Result<(), AppError> {
    Ok(environment.create.open_source(&PathBuf::from(path)).await?)
}

/// Dropped files: a video becomes the source, images become references.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_drop_files(
    environment: Environment<'_>,
    paths: Vec<String>,
) -> Result<AddOutcome, AppError> {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(environment.create.add_files(&paths).await?)
}

/// Add Images…: picks reference images.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_choose_references(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<AddOutcome, AppError> {
    let picked = pick(&app, "Add Images", "Image", &IMAGE_EXTENSIONS, true).await?;
    if picked.is_empty() {
        return Ok(AddOutcome::default());
    }
    Ok(environment.create.add_references(&picked)?)
}

/// Picks the image for a start or end frame.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_choose_frame(
    app: AppHandle,
    environment: Environment<'_>,
    slot: FrameSlot,
) -> Result<(), AppError> {
    let title = match slot {
        FrameSlot::Start => "Choose the Start Frame",
        FrameSlot::End => "Choose the End Frame",
    };
    let picked = pick(&app, title, "Image", &IMAGE_EXTENSIONS, false).await?;
    if let Some(path) = picked.first() {
        environment.create.set_frame(slot, path)?;
    }
    Ok(())
}

/// A pasted image's bytes: saved under `pasted/` and added as a reference.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_paste_image(
    environment: Environment<'_>,
    bytes: Vec<u8>,
) -> Result<AddOutcome, AppError> {
    Ok(environment.create.paste_image(bytes).await?)
}

/// Generate or Draft. `approved` once the person confirmed the price.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_submit(
    environment: Environment<'_>,
    is_draft: bool,
    approved: bool,
) -> Result<SubmitOutcome, AppError> {
    Ok(environment.create.submit(is_draft, approved).await)
}

/// Edit Inputs or Generate again: the job's inputs replace the form.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_load_job(
    environment: Environment<'_>,
    id: String,
) -> Result<LoadOutcome, AppError> {
    Ok(environment.create.load_job(job_id(&id)?).await?)
}

/// Edit… or Extend… on a finished result: the result opens as the source of a Seedance job made from it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_open_follow_up(
    environment: Environment<'_>,
    id: String,
    mode: jutsu_core::Mode,
) -> Result<LoadOutcome, AppError> {
    Ok(environment.create.open_follow_up(job_id(&id)?, mode).await?)
}

/// Undo on the "Draft replaced" toast.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_undo_replacement(environment: Environment<'_>) -> Result<(), AppError> {
    environment.create.undo_replacement();
    Ok(())
}

/// Render final: the draft's inputs at full quality, queued at once.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_render_final(
    environment: Environment<'_>,
    id: String,
    approved: bool,
) -> Result<SubmitOutcome, AppError> {
    Ok(environment.create.render_final(job_id(&id)?, approved).await)
}

/// A prompt picked from the history, or Clear prompt (`""`), replaces the form's prompt. Answers whether Undo can
/// bring the old one back.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_replace_prompt(
    environment: Environment<'_>,
    text: String,
) -> Result<bool, AppError> {
    Ok(environment.create.replace_prompt(&text))
}

/// The style gallery's refresh button.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_refresh_presets(environment: Environment<'_>) -> Result<(), AppError> {
    environment.create.refresh_presets().await;
    Ok(())
}

/// The style gallery's search; answers with the gallery filtered.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_preset_query(
    environment: Environment<'_>,
    query: String,
) -> Result<CreateSnapshot, AppError> {
    Ok(environment.create.set_preset_query(query).await)
}

/// Stars or unstars a style.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_toggle_favorite_preset(
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&id)
        .map_err(|_| AppError::NotFound("This style is no longer in the catalog.".to_owned()))?;
    Ok(environment.create.toggle_favorite_preset(id)?)
}

/// The Motion Library on the empty stage; `None` when the connection has none.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_motion_library(
    environment: Environment<'_>,
) -> Result<Option<MotionLibraryView>, AppError> {
    Ok(environment.create.motion_library())
}

/// Shows a shelf (or the one shown) and loads its first page, or its next one with `more`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_load_motion_library(
    environment: Environment<'_>,
    shelf: Option<String>,
    more: bool,
) -> Result<(), AppError> {
    Ok(environment
        .create
        .load_motion_library(shelf.as_deref(), more)
        .await?)
}

/// Downloads a library clip and opens it as the source.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_use_motion_clip(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    Ok(environment.create.use_motion_clip(&id).await?)
}

/// Add a character: picks the images of one new batch character.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_choose_character_set(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<Option<Notice>, AppError> {
    let picked = pick(&app, "Add a Character", "Image", &IMAGE_EXTENSIONS, true).await?;
    Ok(environment.create.add_character_set(&picked)?)
}

/// Images dropped on the characters: one new batch character.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_character_set(
    environment: Environment<'_>,
    paths: Vec<String>,
) -> Result<Option<Notice>, AppError> {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(environment.create.add_character_set(&paths)?)
}

/// Save to Higgsfield… on a batch character; answers why it failed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_save_character_set(
    environment: Environment<'_>,
    index: i32,
    name: String,
) -> Result<Option<UserMessage>, AppError> {
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    Ok(environment.create.save_character_set(index, &name).await?)
}

/// References › From Higgsfield; `None` without an account connection.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_higgsfield_references(
    environment: Environment<'_>,
) -> Result<Option<HiggsfieldReferencesView>, AppError> {
    Ok(environment.create.higgsfield_references())
}

/// Loads a tab of the sheet: its first page (the uploads again when opened again), or its next with `more`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_load_higgsfield(
    environment: Environment<'_>,
    source: HiggsfieldSource,
    more: bool,
) -> Result<(), AppError> {
    environment.create.load_higgsfield(source, more).await;
    Ok(())
}

/// The Elements tab's Category menu; `None` shows every category.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_set_element_category(
    environment: Environment<'_>,
    category: Option<String>,
) -> Result<(), AppError> {
    Ok(environment.create.set_element_category(category.as_deref())?)
}

/// The sheet closed: a download still running adds nothing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_close_higgsfield(environment: Environment<'_>) -> Result<(), AppError> {
    environment.create.close_higgsfield();
    Ok(())
}

/// Adds the picked uploads in the order picked; answers why it failed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_uploads(
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<Option<String>, AppError> {
    Ok(environment.create.add_uploads(&ids).await?)
}

/// Picks an element: adds its images when they fit, else asks which.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_open_element(
    environment: Environment<'_>,
    id: String,
) -> Result<ElementOpening, AppError> {
    Ok(environment.create.open_element(&id).await?)
}

/// Back from an element's images to the list.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_close_element(environment: Environment<'_>) -> Result<(), AppError> {
    environment.create.close_element();
    Ok(())
}

/// Adds the picked images of the element being chosen from, in the order picked.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_element_images(
    environment: Environment<'_>,
    indices: Vec<u32>,
) -> Result<ElementOpening, AppError> {
    let indices: Vec<usize> = indices
        .into_iter()
        .filter_map(|index| usize::try_from(index).ok())
        .collect();
    Ok(environment.create.add_element_images(&indices).await?)
}

/// Paste URL… in Source (`"video"`) or References (`"image"`): the field, filled with `clipboard` if it is a link.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_link_entry(kind: String, clipboard: String) -> Result<LinkEntryView, AppError> {
    let expected = links::expected_kind(Some(&kind))
        .ok_or_else(|| AppError::Internal(format!("Unknown kind “{kind}”.")))?;
    Ok(links::entry(expected, &clipboard))
}

/// Downloads a link, then takes the file like a dropped one. `expected` is the section's kind; `None` (a pasted link)
/// takes either.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_download_link(
    environment: Environment<'_>,
    text: String,
    expected: Option<String>,
) -> Result<LinkOutcome, AppError> {
    Ok(environment
        .create
        .download_link(&text, links::expected_kind(expected.as_deref()))
        .await?)
}

/// Cancel on the link's download row.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_cancel_link(environment: Environment<'_>) -> Result<(), AppError> {
    environment.create.cancel_link();
    Ok(())
}

/// Add speech or Replace on the Speech row: picks one recording.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_choose_speech(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<Option<Notice>, AppError> {
    let picked = pick(&app, "Add Speech", "Audio", &AUDIO_EXTENSIONS, false).await?;
    Ok(environment.create.add_speech(&picked).await?)
}

/// Files dropped on the Speech row: the first recording becomes the speech.
#[tauri::command]
#[specta::specta]
pub(crate) async fn create_add_speech(
    environment: Environment<'_>,
    paths: Vec<String>,
) -> Result<Option<Notice>, AppError> {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(environment.create.add_speech(&paths).await?)
}
