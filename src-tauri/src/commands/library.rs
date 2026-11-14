//! The Library screen: the grid's snapshot and counts, a result's page, an image's details, and the actions: star,
//! delete, keep and reject with Undo, make primary, the account's history and its downloads, Show in file manager,
//! Open, and saving a result's inputs.

use std::path::{Path, PathBuf};

use jutsu_app::MediaServer;
use jutsu_app::create::SubmitOutcome;
use jutsu_app::create::make_video::{MakeVideoChoice, MakeVideoOutcome};
use jutsu_app::library::actions::BulkActions;
use jutsu_app::library::culling::cursor_after;
use jutsu_app::library::finals::{FinalSection, FinalsSheet, Resolution};
use jutsu_app::library::image_compare::{ImageCompareSides, ImageCompareView, SidesChange};
use jutsu_app::library::outside_edit::IMPORT_MESSAGE;
use jutsu_app::library::view::{BatchView, CompareCandidate, CompareView, ScoreOutcome, ScoreSheet};
use jutsu_app::library::{
    CullingAction, ImageDetails, KindCounts, LibraryError, LibraryRequest, LibrarySnapshot, ResultDetail,
    delete_confirmation, image_trash_confirmation,
};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt;
use time::OffsetDateTime;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

fn material_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This result is no longer in the library.".to_owned()))
}

fn material_ids(ids: &[String]) -> Result<Vec<Uuid>, AppError> {
    ids.iter().map(|id| material_id(id)).collect()
}

fn library_error(error: LibraryError) -> AppError {
    match error {
        LibraryError::Catalog(_) => AppError::Internal(error.to_string()),
        LibraryError::NotFound(message) => AppError::NotFound(message),
    }
}

fn internal(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(error.to_string())
}

/// What the grid shows for `request`. Ask again on `LibraryChanged`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_snapshot(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    request: LibraryRequest,
) -> Result<LibrarySnapshot, AppError> {
    let url_for = |path: &Path| server.url_for(path).ok();
    environment
        .library
        .snapshot(&request, OffsetDateTime::now_utc(), &url_for)
        .await
        .map_err(library_error)
}

/// The tab counts of this computer's Library, for the sidebar.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_counts(environment: Environment<'_>) -> Result<KindCounts, AppError> {
    Ok(environment.library.counts().await)
}

/// Where a card's still frame loads from; `None` without ffmpeg or the file.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_thumbnail(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<Option<String>, AppError> {
    let frame = environment
        .library
        .thumbnail(material_id(&id)?)
        .await
        .map_err(library_error)?;
    Ok(frame.and_then(|path| server.url_for(&path).ok()))
}

/// A file the catalog points at, from wherever it was picked: its folder joins the media server's folders.
fn picked_file_url(server: &MediaServer, path: &Path) -> Option<String> {
    server.url_for(path).ok().or_else(|| {
        server.register_root(path.parent()?);
        server.url_for(path).ok()
    })
}

/// Result `id`'s page; `id` may also be the ID of the job that made it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_detail(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<ResultDetail, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .detail(material_id(&id)?, OffsetDateTime::now_utc(), &url_for)
        .await
        .map_err(library_error)
}

/// Image `id`'s Details…, for `version_id` or its current version.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_image_details(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
    version_id: Option<String>,
) -> Result<ImageDetails, AppError> {
    let material = material_id(&id)?;
    let version = version_id
        .as_deref()
        .and_then(|version| Uuid::parse_str(version).ok());
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .image_details(material, version, &url_for)
        .await
        .map_err(library_error)
}

/// The question Delete from Library… or Move to Trash… asks.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteConfirmation {
    pub title: String,
    pub message: String,
    /// "Delete", or "Move to Trash" for images.
    pub confirm_title: String,
}

/// What Delete from Library… asks before deleting `count` results, or Move to Trash… before `count` images
/// (`images`) go.
#[tauri::command]
#[specta::specta]
pub(crate) fn library_delete_confirmation(count: i32, images: bool) -> DeleteConfirmation {
    let count = usize::try_from(count).unwrap_or(1);
    let ((title, message), confirm_title) = if images {
        (image_trash_confirmation(count), "Move to Trash")
    } else {
        (delete_confirmation(count), "Delete")
    };
    DeleteConfirmation {
        title,
        message,
        confirm_title: confirm_title.to_owned(),
    }
}

/// The question before Remove from Library… takes `count` missing images off the list.
#[tauri::command]
#[specta::specta]
pub(crate) fn library_removal_title(count: i32) -> String {
    jutsu_app::library::actions::image_removal_title(usize::try_from(count).unwrap_or(1))
}

/// Stars or unstars `ids`; on an account, likes them on Higgsfield to match. A refusal shows as the alert.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_set_starred(
    environment: Environment<'_>,
    ids: Vec<String>,
    is_starred: bool,
) -> Result<(), AppError> {
    environment
        .library
        .set_starred(&material_ids(&ids)?, is_starred)
        .await;
    Ok(())
}

/// Delete from Library / Move to Trash: the files go to the trash, the results leave the Library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_delete(environment: Environment<'_>, ids: Vec<String>) -> Result<(), AppError> {
    environment.library.delete(&material_ids(&ids)?).await;
    Ok(())
}

/// Remove from Library, for images whose file is gone.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_remove(environment: Environment<'_>, ids: Vec<String>) -> Result<(), AppError> {
    environment.library.remove(&material_ids(&ids)?).await;
    Ok(())
}

/// What a keep, reject or clear did and where the current card goes.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CullOutcome {
    /// What was done; `None` when nothing was saved.
    pub action: Option<CullingAction>,
    /// The current card after culling `cursor`; `None` without one, or when no card is left.
    pub cursor: Option<String>,
}

/// Keep, reject or clear `ids`. `cursor` is the current card when it alone was culled; `order` the grid's cards and
/// `shows_rejected` whether a rejected result stays on screen, for where the cursor goes next.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_cull(
    environment: Environment<'_>,
    action: CullingAction,
    ids: Vec<String>,
    cursor: Option<String>,
    order: Vec<String>,
    shows_rejected: bool,
) -> Result<CullOutcome, AppError> {
    let action = environment.library.cull(action, &material_ids(&ids)?).await;
    let cursor = match (action, cursor) {
        (Some(action), Some(acted)) => cursor_after(action, &acted, &order, shows_rejected),
        (_, cursor) => cursor,
    };
    Ok(CullOutcome { action, cursor })
}

/// Takes back this session's last keep, reject or clear; returns the result it restored, the new current card,
/// `None` with nothing to undo.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_undo_culling(environment: Environment<'_>) -> Result<Option<String>, AppError> {
    Ok(environment.library.undo_culling().await.map(|id| id.to_string()))
}

/// Make Primary: the variant becomes its group's card.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_make_primary(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    environment.library.make_primary(material_id(&id)?).await;
    Ok(())
}

/// Loads the account's history: the first page again with `refresh`, else the next one.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_load_history(
    environment: Environment<'_>,
    refresh: bool,
) -> Result<(), AppError> {
    environment.library.load_history(refresh).await;
    Ok(())
}

/// Downloads a result or image only on Higgsfield into the Library.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_download(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    environment.library.download(material_id(&id)?).await;
    Ok(())
}

async fn first_file(environment: &Environment<'_>, ids: &[String]) -> Result<PathBuf, AppError> {
    environment
        .library
        .files(&material_ids(ids)?)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("The file is no longer on this computer.".to_owned()))
}

/// Show in file manager for the first of `ids` that is here.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_show_in_folder(
    app: AppHandle,
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<(), AppError> {
    let path = first_file(&environment, &ids).await?;
    app.opener().reveal_item_in_dir(path).map_err(internal)
}

/// Opens result `id` in the system's default app.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_open(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let path = first_file(&environment, &[id]).await?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(internal)
}

/// Save Inputs… (`index` `None`), or Save Source video… on one input's tile (`index` into the page's `inputs`): one
/// file asks where with its name, several pick a folder, where they take unused names (`InputExport`). Shows the
/// saved files in the file manager. Returns how many were saved; 0 when the picker is cancelled.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_save_inputs(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
    index: Option<u32>,
) -> Result<i32, AppError> {
    let id = material_id(&id)?;
    let Some((mut files, base)) = environment.library.inputs_of(id).await else {
        return Ok(0);
    };
    if let Some(index) = index {
        let index = usize::try_from(index).map_err(internal)?;
        if index >= files.len() {
            return Ok(0);
        }
        files = vec![files.swap_remove(index)];
    }
    let targets = match files.as_slice() {
        [] => return Ok(0),
        [only] => {
            let dialog = app.dialog().file().set_file_name(only.file_name(&base));
            let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
                .await
                .map_err(internal)?;
            let Some(picked) = picked else {
                return Ok(0);
            };
            vec![(files.swap_remove(0), picked.into_path().map_err(internal)?)]
        }
        _ => {
            let dialog = app
                .dialog()
                .file()
                .set_title("Choose a folder for the source video and reference images.");
            let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
                .await
                .map_err(internal)?;
            let Some(picked) = picked else {
                return Ok(0);
            };
            let folder = picked.into_path().map_err(internal)?;
            jutsu_app::library::inputs::folder_targets(files, &base, &folder, Path::exists)
        }
    };
    let saved = environment.library.save_inputs(targets).await;
    if let Some(path) = saved.first() {
        app.opener().reveal_item_in_dir(path).map_err(internal)?;
    }
    Ok(i32::try_from(saved.len()).unwrap_or(i32::MAX))
}

/// Show in File Manager on an input's tile: the copy of input `index` (into the page's `inputs`) on this computer.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_reveal_input(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
    index: u32,
) -> Result<(), AppError> {
    let id = material_id(&id)?;
    let path = environment
        .library
        .inputs_of(id)
        .await
        .and_then(|(files, _)| files.into_iter().nth(usize::try_from(index).ok()?))
        .and_then(|file| file.local)
        .filter(|path| path.exists())
        .ok_or_else(|| AppError::NotFound("This input is no longer on this computer.".to_owned()))?;
    app.opener().reveal_item_in_dir(path).map_err(internal)
}

/// Results `ids` side by side on one clock, in that order.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_compare(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    ids: Vec<String>,
) -> Result<CompareView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .compare(&material_ids(&ids)?, &url_for)
        .await
        .map_err(library_error)
}

/// Compare with Original: result `id` beside the version it was made from.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_compare_original(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<CompareView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .compare_original(material_id(&id)?, &url_for)
        .await
        .map_err(library_error)
}

/// What the name field of result `id`'s Save as Template starts with.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_template_name(
    environment: Environment<'_>,
    id: String,
) -> Result<String, AppError> {
    let spec = environment
        .library
        .template_spec(material_id(&id)?, None, OffsetDateTime::now_utc())
        .await
        .map_err(library_error)?;
    Ok(jutsu_app::templates::TemplateService::suggested_name_for(&spec))
}

/// Save as Template… on result `id`: its inputs, a draft's at the final resolution, as a template named `name`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_save_template(
    environment: Environment<'_>,
    id: String,
    name: String,
) -> Result<super::images::TemplateSaveOutcome, AppError> {
    let spec = environment
        .library
        .template_spec(material_id(&id)?, None, OffsetDateTime::now_utc())
        .await
        .map_err(library_error)?;
    Ok(match environment.templates.save_spec(&spec, &name).await {
        Ok(name) => super::images::TemplateSaveOutcome::Saved { name },
        Err(error) => super::images::TemplateSaveOutcome::Failed {
            notice: jutsu_app::templates::not_saved(&error),
        },
    })
}

/// The Library's Compare pick bar after `toggled` is picked or unpicked; `None` starts picking.
#[tauri::command]
#[specta::specta]
pub(crate) fn library_compare_pick(
    picked: Vec<String>,
    toggled: Option<String>,
) -> jutsu_app::library::compare::ComparePick {
    jutsu_app::library::compare::ComparePick::new(picked, toggled)
}

/// The two images picked to compare, side by side at the same zoom, the first on the left.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_image_compare(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    ids: Vec<String>,
) -> Result<ImageCompareView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .image_compare(&material_ids(&ids)?, &url_for)
        .await
        .map_err(library_error)
}

/// Compare with Original on image `id`: the earlier version it was made from on the left, the image on the right.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_image_compare_original(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<ImageCompareView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .image_compare_original(material_id(&id)?, &url_for)
        .await
        .map_err(library_error)
}

/// Compare Variants on an image: variant group `group` as the grid shows it for `request`, two at a time.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_image_variants(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    request: LibraryRequest,
    group: String,
) -> Result<ImageCompareView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .image_variants(&request, material_id(&group)?, &url_for)
        .await
        .map_err(library_error)
}

/// The image compare's sides after `change` among `count` images.
#[tauri::command]
#[specta::specta]
pub(crate) fn library_image_compare_sides(
    sides: ImageCompareSides,
    count: i32,
    change: SidesChange,
) -> ImageCompareSides {
    sides.changed(usize::try_from(count).unwrap_or(0), change)
}

/// Copy Image: image `id`'s picture on the clipboard.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_copy_image(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let pixels = environment
        .library
        .image_pixels(material_id(&id)?)
        .await
        .ok_or_else(|| AppError::NotFound("The image is no longer on this computer.".to_owned()))?;
    let image = tauri::image::Image::new_owned(pixels.rgba, pixels.width, pixels.height);
    app.clipboard().write_image(&image).map_err(internal)
}

/// What "Compare…" offers for result `id`: the same source first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_compare_candidates(
    environment: Environment<'_>,
    id: String,
) -> Result<Vec<CompareCandidate>, AppError> {
    environment
        .library
        .compare_candidates(material_id(&id)?)
        .await
        .map_err(library_error)
}

/// Batch `id` as a comparison.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_batch(
    environment: Environment<'_>,
    server: State<'_, MediaServer>,
    id: String,
) -> Result<BatchView, AppError> {
    let url_for = |path: &Path| picked_file_url(&server, path);
    environment
        .library
        .batch(material_id(&id)?, OffsetDateTime::now_utc(), &url_for)
        .await
        .map_err(library_error)
}

/// The Score Variants sheet for results `ids`: what scoring them costs.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_score_sheet(
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<ScoreSheet, AppError> {
    environment
        .library
        .score_sheet(&material_ids(&ids)?)
        .await
        .map_err(library_error)
}

/// Scores results `ids` in the background; `approved` once the person agreed to the price under the cap.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_score(
    environment: Environment<'_>,
    ids: Vec<String>,
    approved: bool,
) -> Result<ScoreOutcome, AppError> {
    environment
        .library
        .start_scoring(&material_ids(&ids)?, approved)
        .await
        .map_err(library_error)
}

/// The "Render final" box of result `id` at `resolution`, or the one it opens on; `None` when it has none.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_final_section(
    environment: Environment<'_>,
    id: String,
    resolution: Option<Resolution>,
) -> Result<Option<FinalSection>, AppError> {
    environment
        .library
        .final_section(material_id(&id)?, resolution, OffsetDateTime::now_utc())
        .await
        .map_err(library_error)
}

/// Render final or Finalize in 1080p of draft `id`; `approved` once the person agreed to go over the cap.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_render_final(
    environment: Environment<'_>,
    id: String,
    resolution: Resolution,
    approved: bool,
) -> Result<SubmitOutcome, AppError> {
    Ok(environment
        .library
        .render_final(material_id(&id)?, resolution, approved)
        .await)
}

/// The Render finals for starred sheet for results `ids` at `resolution`, or the one it opens on.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_finals_sheet(
    environment: Environment<'_>,
    ids: Vec<String>,
    resolution: Option<Resolution>,
) -> Result<FinalsSheet, AppError> {
    environment
        .library
        .finals_sheet(&material_ids(&ids)?, resolution)
        .await
        .map_err(library_error)
}

/// Queues the finals of the starred drafts among `ids` at `resolution`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_render_finals(
    environment: Environment<'_>,
    ids: Vec<String>,
    resolution: Resolution,
    approved: bool,
) -> Result<SubmitOutcome, AppError> {
    Ok(environment
        .library
        .render_finals(&material_ids(&ids)?, resolution, approved)
        .await)
}

/// Asks where to save an MP4 named `name`; `None` when cancelled.
async fn save_mp4(app: &AppHandle, name: String) -> Result<Option<PathBuf>, AppError> {
    let dialog = app
        .dialog()
        .file()
        .set_file_name(name)
        .add_filter("MPEG-4 movie", &["mp4"]);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(internal)?;
    picked.map(|path| path.into_path().map_err(internal)).transpose()
}

/// Join with Original…: asks where to save, then writes the source selection and the Extend as one MP4 and shows
/// it in the file manager. A failure shows as the alert.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_join_with_original(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let id = material_id(&id)?;
    let name = environment.library.join_name(id).await.map_err(library_error)?;
    let Some(output) = save_mp4(&app, name).await? else {
        return Ok(());
    };
    environment.library.join_with_original(id, &output).await;
    if output.exists() {
        app.opener().reveal_item_in_dir(output).map_err(internal)?;
    }
    Ok(())
}

/// Export Short Preview…: asks where to save, then writes the first 5 seconds at 480p without sound and shows it
/// in the file manager. A failure shows as the alert.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_export_short_preview(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let id = material_id(&id)?;
    let name = environment
        .library
        .preview_name(id)
        .await
        .map_err(library_error)?;
    let Some(output) = save_mp4(&app, name).await? else {
        return Ok(());
    };
    environment.library.export_short_preview(id, &output).await;
    if output.exists() {
        app.opener().reveal_item_in_dir(output).map_err(internal)?;
    }
    Ok(())
}

/// The files of `ids` that are here, for Copy Videos and Share.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_files(
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<Vec<String>, AppError> {
    Ok(environment
        .library
        .files(&material_ids(&ids)?)
        .await
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect())
}

/// Edit in…: opens a working copy of result or image `id` in the system's default app for its type; the original
/// stays as it is.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_edit_in(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    if let Some(copy) = environment.library.edit_in(material_id(&id)?).await {
        app.opener()
            .open_path(copy.to_string_lossy(), None::<&str>)
            .map_err(internal)?;
    }
    Ok(())
}

/// Import as New Version…: picks the edited file and copies it into the library as the new version of `id`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_import_version(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let id = material_id(&id)?;
    let dialog = app.dialog().file().set_title(IMPORT_MESSAGE);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .map_err(internal)?;
    let Some(picked) = picked else {
        return Ok(());
    };
    let file = picked.into_path().map_err(internal)?;
    environment.library.import_version(id, &file).await;
    Ok(())
}

/// Make Video… from images `ids`: they go into Create in the role the selected model takes, asked first when more
/// than one fits; send the picked `choice` back. With `as_references`, Use as Reference: they become Create's
/// references. Images only on Higgsfield are downloaded first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_make_video(
    environment: Environment<'_>,
    ids: Vec<String>,
    as_references: bool,
    choice: Option<MakeVideoChoice>,
) -> Result<MakeVideoOutcome, AppError> {
    let files = environment.library.image_files(&material_ids(&ids)?).await;
    if files.is_empty() {
        return Err(AppError::NotFound(
            "The image is no longer on this computer.".to_owned(),
        ));
    }
    if as_references {
        let notice = environment.create.add_files(&files).await?.notice;
        return Ok(MakeVideoOutcome::Placed {
            can_undo: false,
            notice: notice.map(|notice| {
                jutsu_app::connection::UserMessage::new(
                    jutsu_app::library::actions::REFERENCES_NOT_ADDED,
                    notice,
                    None,
                )
            }),
        });
    }
    Ok(environment.create.make_video(&files, choice).await?)
}

/// What the selection bar can do with `ids`: which can be deleted, and whether the star button stars or unstars.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_bulk(
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<BulkActions, AppError> {
    environment
        .library
        .bulk(&material_ids(&ids)?)
        .await
        .map_err(library_error)
}

/// Star for several results: stars them all unless every one is starred, else unstars them.
#[tauri::command]
#[specta::specta]
pub(crate) async fn library_toggle_star(
    environment: Environment<'_>,
    ids: Vec<String>,
) -> Result<(), AppError> {
    environment.library.toggle_star(&material_ids(&ids)?).await;
    Ok(())
}
