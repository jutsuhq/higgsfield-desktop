//! The window around the pages: the menu bar's state, the About window, the sidebar's layout, and files opened from
//! outside or with File › Open….

use std::path::PathBuf;

use jutsu_app::system::about::{About, REPOSITORY, about};
use jutsu_app::system::app_menu::PageCommandState;
use jutsu_app::system::opened_files::{
    IMAGE_EXTENSIONS, OpenRequest, OpenedFilesAlert, VIDEO_EXTENSIONS, open_in_create,
};
use jutsu_app::system::sidebar::{SidebarLayout, SidebarStore};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use super::Environment;
use crate::error::AppError;
use crate::system;

/// The commands the page on screen carries out, with their titles: the menu bar enables them and runs them there.
#[tauri::command]
#[specta::specta]
pub(crate) async fn app_menu_publish(
    app: AppHandle,
    environment: Environment<'_>,
    commands: Vec<PageCommandState>,
) -> Result<(), AppError> {
    system::app_menu::publish(&app, &environment, commands).await;
    Ok(())
}

/// What the About window shows.
#[tauri::command]
#[specta::specta]
pub(crate) fn about_content() -> About {
    about(Some(env!("CARGO_PKG_VERSION")))
}

/// The About window's repository link, in the browser.
#[tauri::command]
#[specta::specta]
pub(crate) async fn about_open_repository(app: AppHandle) -> Result<(), AppError> {
    app.opener()
        .open_url(REPOSITORY, None::<&str>)
        .map_err(|error| AppError::Internal(error.to_string()))
}

/// The sidebar as it was left.
#[tauri::command]
#[specta::specta]
pub(crate) async fn sidebar_layout(environment: Environment<'_>) -> Result<SidebarLayout, AppError> {
    Ok(SidebarStore(&environment.preferences).layout())
}

/// Shows or hides the sidebar, or keeps the width it was dragged to.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_sidebar_layout(
    environment: Environment<'_>,
    is_collapsed: Option<bool>,
    width: Option<i32>,
) -> Result<SidebarLayout, AppError> {
    SidebarStore(&environment.preferences)
        .set(is_collapsed, width)
        .map_err(|error| AppError::Internal(error.to_string()))
}

/// File › Open…: videos and images; a video becomes Create's source, images its references. The window goes to Create,
/// which takes them.
#[tauri::command]
#[specta::specta]
pub(crate) async fn opened_files_choose(app: AppHandle) -> Result<(), AppError> {
    let extensions: Vec<&str> = VIDEO_EXTENSIONS
        .iter()
        .chain(&IMAGE_EXTENSIONS)
        .copied()
        .collect();
    let dialog = app
        .dialog()
        .file()
        .set_title("Open")
        .add_filter("Videos and images", &extensions);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_files())
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
        .unwrap_or_default();
    let paths = picked
        .into_iter()
        .map(|file| {
            file.into_path()
                .map_err(|error| AppError::Internal(error.to_string()))
        })
        .collect::<Result<Vec<PathBuf>, AppError>>()?;
    system::opened::open(&app, OpenRequest::new(paths));
    Ok(())
}

/// Create is on screen: it takes the files waiting for it. Answers with the alert to show, if any.
#[tauri::command]
#[specta::specta]
pub(crate) async fn opened_files_take(
    app: AppHandle,
    environment: Environment<'_>,
) -> Result<Option<OpenedFilesAlert>, AppError> {
    let Some(request) = system::opened::pending(&app).take() else {
        return Ok(None);
    };
    Ok(open_in_create(&environment.create, request).await)
}
