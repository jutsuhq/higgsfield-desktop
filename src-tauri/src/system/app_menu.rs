//! The main window's menu bar (`AppCommands.swift`). The window keeps the system's title bar, so the menu is the
//! window's own native menu, as Windows and Linux apps show it. What it lists comes from `jutsu_app::system::app_menu`;
//! this makes it, changes titles and states in place, and carries out what is chosen.
//!
//! Its shortcuts are the menu's accelerators. Where the system hands them to the menu (Linux), choosing by key comes
//! here like a click and runs the page's handler; where the web view keeps the keys (Windows), the page's own key
//! handler runs the same handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use jutsu_app::AppEnvironment;
use jutsu_app::settings::ShortcutStore;
use jutsu_app::system::app_menu::{
    self, AppMenuAction, AppMenuInput, AppMenuItem, AppMenuSection, MenuTemplate, PageCommandState,
};
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};
use tauri_specta::Event as _;
use uuid::Uuid;

use super::{about, environment, updater, window};
use crate::events::MenuCommandRequested;

/// What the menu shows now, and what made it.
#[derive(Default)]
struct MenuState {
    page_commands: Vec<PageCommandState>,
    templates: Vec<MenuTemplate>,
    shown: Option<Vec<AppMenuSection>>,
    items: HashMap<String, MenuItem<Wry>>,
}

fn state(app: &AppHandle) -> Arc<Mutex<MenuState>> {
    if let Some(state) = app.try_state::<Arc<Mutex<MenuState>>>() {
        return state.inner().clone();
    }
    let state = Arc::new(Mutex::new(MenuState::default()));
    app.manage(state.clone());
    state
}

/// Follows the menu's items on the main window.
pub(crate) fn start(app: &AppHandle) {
    if let Some(main) = app.get_webview_window(window::MAIN) {
        main.on_menu_event(|window, event| on_menu(window.app_handle(), event.id().as_ref()));
    }
}

/// The commands the page on screen publishes, then the menu as things stand.
pub(crate) async fn publish(
    app: &AppHandle,
    environment: &Arc<AppEnvironment>,
    commands: Vec<PageCommandState>,
) {
    let side = environment.images.side();
    let templates = match environment.templates.list(side).await {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                Some(MenuTemplate {
                    id: Uuid::parse_str(&row.id).ok()?,
                    name: row.name,
                })
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "can't list the templates for the menu");
            Vec::new()
        }
    };
    {
        let state = state(app);
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        state.page_commands = commands;
        state.templates = templates;
    }
    refresh(app, environment);
}

/// Makes the menu as things stand and shows what changed. Menus are made on the main thread.
pub(crate) fn refresh(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    let sections = {
        let state = state(app);
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let shortcuts = ShortcutStore(&environment.preferences).assigned();
        let (can_pause_all, can_resume_all) = environment
            .queue
            .with_live(|live| (live.can_pause_all(), live.can_resume_all()));
        app_menu::app_menu(&AppMenuInput {
            page_commands: &state.page_commands,
            shortcuts: &shortcuts,
            can_pause_all,
            can_resume_all,
            can_check_for_updates: updater::availability(app).is_available() && !updater::is_busy(app),
            templates: &state.templates,
        })
    };
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || apply(&handle, sections)) {
        tracing::error!(%error, "can't update the menu");
    }
}

fn apply(app: &AppHandle, sections: Vec<AppMenuSection>) {
    let state = state(app);
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(shown) = state.shown.as_ref() else {
        rebuild(app, &mut state, sections);
        return;
    };
    if *shown == sections {
        return;
    }
    if !app_menu::same_layout(shown, &sections) {
        rebuild(app, &mut state, sections);
        return;
    }
    for section in &sections {
        patch(&state.items, &section.items);
    }
    state.shown = Some(sections);
}

fn rebuild(app: &AppHandle, state: &mut MenuState, sections: Vec<AppMenuSection>) {
    let Some(main) = app.get_webview_window(window::MAIN) else {
        return;
    };
    let mut items = HashMap::new();
    match menu(app, &sections, &mut items).and_then(|menu| main.set_menu(menu)) {
        Ok(_) => {
            state.items = items;
            state.shown = Some(sections);
        }
        Err(error) => tracing::error!(%error, "can't make the menu"),
    }
}

/// Changes titles, enabled states and shortcuts in place.
fn patch(made: &HashMap<String, MenuItem<Wry>>, items: &[AppMenuItem]) {
    for item in items {
        match item {
            AppMenuItem::Item {
                action,
                title,
                is_enabled,
                accelerator,
            } => {
                let Some(made) = made.get(&action.id()) else {
                    continue;
                };
                let result = made
                    .set_text(title)
                    .and_then(|()| made.set_enabled(*is_enabled))
                    .and_then(|()| made.set_accelerator(accelerator.as_deref()));
                if let Err(error) = result {
                    tracing::warn!(%error, "can't change a menu item");
                }
            }
            AppMenuItem::Submenu { items, .. } => patch(made, items),
            AppMenuItem::Label(_) | AppMenuItem::Separator => {}
        }
    }
}

fn menu(
    app: &AppHandle,
    sections: &[AppMenuSection],
    made: &mut HashMap<String, MenuItem<Wry>>,
) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    for section in sections {
        let children = entries(app, &section.items, made)?;
        let references: Vec<&dyn IsMenuItem<Wry>> = children.iter().map(AsRef::as_ref).collect();
        menu.append(&Submenu::with_items(app, &section.title, true, &references)?)?;
    }
    Ok(menu)
}

fn entries(
    app: &AppHandle,
    items: &[AppMenuItem],
    made: &mut HashMap<String, MenuItem<Wry>>,
) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut list: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for item in items {
        match item {
            AppMenuItem::Item {
                action,
                title,
                is_enabled,
                accelerator,
            } => {
                let id = action.id();
                // A shortcut the menu can't read leaves the item without one rather than without the item.
                let item = MenuItem::with_id(app, &id, title, *is_enabled, accelerator.as_deref())
                    .or_else(|_| MenuItem::with_id(app, &id, title, *is_enabled, None::<&str>))?;
                made.insert(id, item.clone());
                list.push(Box::new(item));
            }
            AppMenuItem::Label(text) => list.push(Box::new(MenuItem::new(app, text, false, None::<&str>)?)),
            AppMenuItem::Separator => list.push(Box::new(PredefinedMenuItem::separator(app)?)),
            AppMenuItem::Submenu { title, items } => {
                let children = entries(app, items, made)?;
                let references: Vec<&dyn IsMenuItem<Wry>> = children.iter().map(AsRef::as_ref).collect();
                list.push(Box::new(Submenu::with_items(app, title, true, &references)?));
            }
        }
    }
    Ok(list)
}

fn on_menu(app: &AppHandle, id: &str) {
    let Some(action) = AppMenuAction::from_id(id) else {
        return;
    };
    let request = |command, template| {
        if let Err(error) = (MenuCommandRequested { command, template }).emit(app) {
            tracing::error!(%error, "can't send the menu's command to the window");
        }
    };
    match action {
        AppMenuAction::About => about::open(app),
        AppMenuAction::Quit => window::quit(app),
        AppMenuAction::Page(command) => request(Some(command), None),
        AppMenuAction::Template(id) => request(None, Some(id.to_string())),
        AppMenuAction::CheckForUpdates => {
            let Some(environment) = environment(app) else {
                return;
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move { updater::check_from_menu(&app, &environment).await });
        }
        AppMenuAction::PauseAll | AppMenuAction::ResumeAll => {
            let Some(environment) = environment(app) else {
                return;
            };
            tauri::async_runtime::spawn(async move {
                if action == AppMenuAction::PauseAll {
                    environment.queue.pause_all().await;
                } else {
                    environment.queue.resume_all().await;
                }
            });
        }
    }
}
