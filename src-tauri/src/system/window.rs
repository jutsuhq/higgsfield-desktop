//! The main window and the app's life (`AppDelegate`, `WindowLifecycle`): opening it on a page, what closing it does,
//! a start at login hidden in tray mode, and Quit's question while work runs.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use jutsu_app::AppEnvironment;
use jutsu_app::system::background::{self, QuitReason};
use jutsu_app::system::tray::TrayOpen;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use tauri_specta::Event as _;

use super::{environment, panel};
use crate::events::AgentShowRequested;

/// The main window's label.
pub(crate) const MAIN: &str = "main";

/// Restores the main window's saved size and place, then centers it when its title bar is off every display.
pub(crate) fn restore_placement(app: &AppHandle) {
    use jutsu_app::system::window_placement::{Frame, can_be_grabbed};
    use tauri_plugin_window_state::{StateFlags, WindowExt as _};
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    if let Err(error) = window.restore_state(StateFlags::all()) {
        tracing::warn!(%error, "can't restore the window's size and place");
    }
    let (Ok(position), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return;
    };
    let screens: Vec<Frame> = monitors
        .iter()
        .map(|monitor| Frame {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
        })
        .collect();
    let frame = Frame {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };
    if !can_be_grabbed(frame, &screens) {
        tracing::info!(
            ?frame,
            "the saved window position is off screen; centering the window"
        );
        let _ = window.center();
    }
}

/// Brings the main window forward on `place`.
pub(crate) fn open(app: &AppHandle, place: &TrayOpen) {
    panel::hide(app);
    crate::bring_to_front(app);
    if let Some(target) = place.target()
        && let Err(error) = (AgentShowRequested {
            route: target.route(),
        })
        .emit(app)
    {
        tracing::error!(%error, "can't send the window to its page");
    }
}

/// The main window was asked to close. `true` when this handled it, so the window must stay: hidden while the app
/// keeps running in the tray or for its work, or kept while the app quits.
pub(crate) fn close_main(app: &AppHandle) -> bool {
    let Some(environment) = environment(app) else {
        return false;
    };
    if jutsu_app::system::keeps_running_without_window(&environment) {
        // With no tray icon the app quits once the work is done, unless the window is opened again.
        let preferences = environment.preferences.get();
        if background::quits_when_work_ends(
            preferences
                .show_in_menu_bar
                .unwrap_or(jutsu_app::Preferences::DEFAULT_SHOW_IN_MENU_BAR),
            preferences.runs_in_menu_bar(),
        ) {
            crate::HIDDEN_FOR_JOBS.store(true, Ordering::SeqCst);
        }
        if let Some(window) = app.get_webview_window(MAIN) {
            let _ = window.hide();
        }
        tracing::info!("the window was closed; the app keeps running");
        return true;
    }
    if app.get_webview_window(panel::LABEL).is_some() {
        // The hidden panel would keep the app running with no window to show.
        app.exit(0);
        return true;
    }
    false
}

/// A start at login in tray mode, or by the `jutsu` helper, shows no window (`MenuBarMode.startsHidden`).
pub(crate) fn hide_at_launch(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    let arguments: Vec<String> = std::env::args().collect();
    let launch = background::launch(&arguments);
    if background::starts_hidden(launch, environment.preferences.get().runs_in_menu_bar())
        && let Some(window) = app.get_webview_window(MAIN)
    {
        tracing::info!(?launch, "starting without the window");
        let _ = window.hide();
    }
}

/// Quit from the tray menu or the panel: at once when nothing runs, else after the person confirms.
pub(crate) fn quit(app: &AppHandle) {
    let prompt = environment(app)
        .and_then(|environment| jutsu_app::system::quit_prompt(&environment, QuitReason::User));
    let Some(prompt) = prompt else {
        app.exit(0);
        return;
    };
    let handle = app.clone();
    app.dialog()
        .message(prompt.detail)
        .title(prompt.message)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit".to_owned(),
            "Cancel".to_owned(),
        ))
        .show(move |quits| {
            if quits {
                handle.exit(0);
            }
        });
}
