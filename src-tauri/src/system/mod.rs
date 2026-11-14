//! The app outside its window (`JutsuStudio/System`): the tray icon, its menu and panel, job notifications, the
//! taskbar's progress and badge, what closing the window and quitting do, Software Update and the microphone on
//! Linux. What each shows and decides comes from `jutsu_app::system`; this applies it.

pub(crate) mod about;
pub(crate) mod app_menu;
#[cfg(target_os = "linux")]
mod media;
mod notifications;
pub(crate) mod opened;
pub(crate) mod panel;
mod taskbar;
mod tray;
pub(crate) mod updater;
pub(crate) mod window;

use std::sync::Arc;

use jutsu_app::AppEnvironment;
use jutsu_app::system::JobNotifier;
use tauri::{AppHandle, Manager};
use time::OffsetDateTime;

/// The environment, once `setup` made it.
pub(crate) fn environment(app: &AppHandle) -> Option<Arc<AppEnvironment>> {
    app.try_state::<Arc<AppEnvironment>>()
        .map(|state| state.inner().clone())
}

/// Starts everything outside the window: hides it for a start at login in tray mode, makes the tray icon, registers
/// the notifications' buttons, and follows the queue and the settings until the app quits.
pub(crate) fn start(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    window::restore_placement(app);
    window::hide_at_launch(app, environment);
    #[cfg(target_os = "linux")]
    media::allow_microphone(app);
    notifications::register(app);
    if let Err(error) = tray::create(app, environment) {
        tracing::error!(%error, "can't make the tray icon");
    }
    taskbar::apply(app, environment);
    app_menu::start(app);
    updater::start(app, environment);
    follow(app, environment);
}

/// After every change to the queue, its progress or the settings: posts what finished or needs the person, and
/// redraws the tray icon, its menu and the taskbar.
fn follow(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    let notifier = Arc::new(JobNotifier::new(OffsetDateTime::now_utc()));

    let (handle, env) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move { refresh(&handle, &env).await });

    let mut queue = environment.queue.subscribe();
    let (handle, env, posts) = (app.clone(), environment.clone(), notifier.clone());
    tauri::async_runtime::spawn(async move {
        while queue.changed().await.is_ok() {
            for notification in posts.changes(&env) {
                notifications::post(&handle, &notification);
            }
            refresh(&handle, &env).await;
        }
    });

    let mut progress = environment.queue.subscribe_progress();
    let (handle, env) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            match progress.recv().await {
                Ok(_) => refresh(&handle, &env).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let mut preferences = environment.preferences.subscribe();
    let (handle, env) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move {
        while preferences.changed().await.is_ok() {
            refresh(&handle, &env).await;
        }
    });
}

/// Redraws the tray and the taskbar; each skips what didn't change.
async fn refresh(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    let panel_menu = if tray::MENU_STANDS_FOR_PANEL {
        let panel = panel::model(app, environment).await;
        jutsu_app::system::tray::panel_menu(&panel)
    } else {
        Vec::new()
    };
    let (handle, env) = (app.clone(), environment.clone());
    // Menus and icons are made on the main thread.
    let posted = app.run_on_main_thread(move || {
        tray::update(&handle, &env, panel_menu);
        taskbar::apply(&handle, &env);
    });
    app_menu::refresh(app, environment);
    if let Err(error) = posted {
        tracing::error!(%error, "can't redraw the tray icon");
    }
}
