//! The tray icon (`MenuBarIcon`, `DockMenu`): its picture with the work's progress, its tooltip and count, and its
//! menu. A left click opens the panel where the platform reports one (Windows, macOS); Linux's tray reports no click,
//! so there the menu also carries the panel's rows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use jutsu_app::AppEnvironment;
use jutsu_app::system::tray::{PanelAction, TrayCommand, TrayMenuItem};
use jutsu_app::system::{TrayStatus, icons, tray_status};
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use time::OffsetDateTime;

use super::{environment, panel, window};

const ID: &str = "main";

/// Whether the menu carries the panel's rows: where the tray icon reports no click.
pub(crate) const MENU_STANDS_FOR_PANEL: bool = cfg!(target_os = "linux");

/// What the tray shows now, so a redraw changes only what moved; and what the panel's menu items do.
#[derive(Default)]
struct Shown {
    status: Option<TrayStatus>,
    panel_menu: Vec<TrayMenuItem>,
    actions: HashMap<String, PanelAction>,
}

fn shown(app: &AppHandle) -> Arc<Mutex<Shown>> {
    if let Some(state) = app.try_state::<Arc<Mutex<Shown>>>() {
        return state.inner().clone();
    }
    let state = Arc::new(Mutex::new(Shown::default()));
    app.manage(state.clone());
    state
}

/// Makes the tray icon as things stand.
pub(crate) fn create(app: &AppHandle, environment: &Arc<AppEnvironment>) -> tauri::Result<()> {
    let status = tray_status(environment, OffsetDateTime::now_utc());
    let mut actions = HashMap::new();
    let mut builder = TrayIconBuilder::with_id(ID)
        .tooltip(&status.tooltip)
        .menu(&menu(app, &status.menu, &[], &mut actions)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(on_icon);
    if let Some(icon) = icon(app, status.progress) {
        builder = builder.icon(icon);
    }
    let tray = builder.build(app)?;
    tray.set_title(status.title.as_deref())?;
    tray.set_visible(status.shows_icon)?;
    let state = shown(app);
    let mut shown = state.lock().unwrap_or_else(PoisonError::into_inner);
    shown.status = Some(status);
    shown.actions = actions;
    Ok(())
}

/// Redraws what changed since the last call. Runs on the main thread.
pub(crate) fn update(app: &AppHandle, environment: &Arc<AppEnvironment>, panel_menu: Vec<TrayMenuItem>) {
    let Some(tray) = app.tray_by_id(ID) else {
        return;
    };
    let status = tray_status(environment, OffsetDateTime::now_utc());
    let state = shown(app);
    let mut shown = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let Err(error) = apply(app, &tray, &shown, &status, &panel_menu).map(|actions| {
        if let Some(actions) = actions {
            shown.actions = actions;
        }
    }) {
        tracing::error!(%error, "can't redraw the tray icon");
    }
    shown.status = Some(status);
    shown.panel_menu = panel_menu;
}

/// Applies `status` where it differs from what shows; returns the panel's menu actions when the menu was made again.
fn apply(
    app: &AppHandle,
    tray: &TrayIcon,
    shown: &Shown,
    status: &TrayStatus,
    panel_menu: &[TrayMenuItem],
) -> tauri::Result<Option<HashMap<String, PanelAction>>> {
    let before = shown.status.as_ref();
    if before.map(|before| before.shows_icon) != Some(status.shows_icon) {
        tray.set_visible(status.shows_icon)?;
    }
    if before.map(|before| &before.tooltip) != Some(&status.tooltip) {
        tray.set_tooltip(Some(&status.tooltip))?;
    }
    if before.map(|before| &before.title) != Some(&status.title) {
        tray.set_title(status.title.as_deref())?;
    }
    if before.map(|before| before.progress) != Some(status.progress) {
        tray.set_icon(icon(app, status.progress))?;
    }
    if before.map(|before| &before.menu) == Some(&status.menu) && shown.panel_menu == panel_menu {
        return Ok(None);
    }
    let mut actions = HashMap::new();
    tray.set_menu(Some(menu(app, &status.menu, panel_menu, &mut actions)?))?;
    Ok(Some(actions))
}

/// The app's icon, with a progress bar while work runs.
fn icon(app: &AppHandle, progress: Option<f64>) -> Option<Image<'static>> {
    let base = app.default_window_icon()?;
    let rgba = match progress {
        Some(fraction) => icons::with_progress(base.rgba(), base.width(), base.height(), fraction),
        None => base.rgba().to_vec(),
    };
    Some(Image::new_owned(rgba, base.width(), base.height()))
}

/// The menu: the panel's rows where they stand in for it, then the app's commands. Each panel item gets an ID whose
/// action goes into `actions`.
fn menu(
    app: &AppHandle,
    commands: &[TrayMenuItem],
    panel_menu: &[TrayMenuItem],
    actions: &mut HashMap<String, PanelAction>,
) -> tauri::Result<Menu<Wry>> {
    let items = entries(app, panel_menu.iter().chain(commands), actions)?;
    let references: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(AsRef::as_ref).collect();
    Menu::with_items(app, &references)
}

fn entries<'a>(
    app: &AppHandle,
    items: impl Iterator<Item = &'a TrayMenuItem>,
    actions: &mut HashMap<String, PanelAction>,
) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut made: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for item in items {
        match item {
            TrayMenuItem::Item {
                command,
                title,
                is_enabled,
            } => made.push(Box::new(MenuItem::with_id(
                app,
                command.id(),
                title,
                *is_enabled,
                None::<&str>,
            )?)),
            TrayMenuItem::Separator => made.push(Box::new(PredefinedMenuItem::separator(app)?)),
            TrayMenuItem::Section(title) => {
                made.push(Box::new(MenuItem::new(app, title, false, None::<&str>)?));
            }
            TrayMenuItem::Action {
                action,
                title,
                is_enabled,
            } => {
                let id = format!("tray.panel.{}", actions.len());
                actions.insert(id.clone(), action.clone());
                made.push(Box::new(MenuItem::with_id(
                    app,
                    id,
                    title,
                    *is_enabled,
                    None::<&str>,
                )?));
            }
            TrayMenuItem::Submenu { title, items } => {
                let children = entries(app, items.iter(), actions)?;
                let references: Vec<&dyn IsMenuItem<Wry>> = children.iter().map(AsRef::as_ref).collect();
                made.push(Box::new(Submenu::with_items(app, title, true, &references)?));
            }
        }
    }
    Ok(made)
}

#[allow(clippy::needless_pass_by_value, reason = "the shape Tauri calls")]
fn on_icon(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        rect,
        ..
    } = event
    {
        panel::toggle(tray.app_handle(), rect);
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    if let Some(command) = TrayCommand::from_id(id) {
        if let Some(place) = command.opens() {
            window::open(app, &place);
            return;
        }
        let Some(environment) = environment(app) else {
            return;
        };
        match command {
            TrayCommand::PauseAll => {
                tauri::async_runtime::spawn(async move { environment.queue.pause_all().await });
            }
            TrayCommand::ResumeAll => {
                tauri::async_runtime::spawn(async move { environment.queue.resume_all().await });
            }
            TrayCommand::Quit => window::quit(app),
            TrayCommand::Open | TrayCommand::Queue | TrayCommand::Library | TrayCommand::Settings => {}
        }
        return;
    }
    let action = shown(app)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .actions
        .get(id)
        .cloned();
    if let (Some(action), Some(environment)) = (action, environment(app)) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { panel::perform(&app, &environment, action).await });
    }
}
