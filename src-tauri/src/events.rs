//! Typed events Rust sends the React app, and the tasks that send them when the state behind them changes.

use std::sync::Arc;

use jutsu_app::connection::ConnectionSnapshot;
use jutsu_app::{AppEnvironment, MediaServer, Preferences};
use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;
use tauri_specta::Event;

/// The connection changed: connected, disconnected, signing in, a new balance, an alert.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct ConnectionChanged {
    pub snapshot: ConnectionSnapshot,
}

/// A setting changed, from this window or another part of the app.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct PreferencesChanged {
    pub preferences: Preferences,
}

/// A job changed state, or something else the Queue lists changed: ask for `queue_snapshot` again. Changes that
/// come close together send one event.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct QueueChanged {
    /// Jobs in the queue now, paused ones included: the sidebar badge.
    pub active_count: i32,
}

/// The Create form or what it shows changed: a quote came back, a proxy progressed, a draft was restored. Ask for
/// `create_snapshot` again.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct CreateChanged {
    pub revision: i32,
}

/// The catalog or what the Library shows of it changed: a result finished, a star, a keep or reject, a history
/// page. Ask for `library_snapshot` and `library_counts` again.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct LibraryChanged {
    pub revision: i32,
}

/// The Image side of Create changed: a run moved on, an image arrived or was saved, a price came back. Ask for
/// `images_snapshot` again.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct ImagesChanged {
    pub revision: i32,
}

/// A run on this computer started, moved on or ended. Ask for `on_device_runs` again.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct OnDeviceRunsChanged {
    pub revision: i32,
}

/// A subtitles run, the speech model or the languages changed. Ask for the sheet or the settings again.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct SubtitlesChanged {
    pub revision: i32,
}

/// An agent asked to `show` a page, a job or an item's sheet: the window routes there. The window was brought forward
/// already.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct AgentShowRequested {
    pub route: jutsu_app::agents::service::AgentShowRoute,
}

/// An item of the window's menu was chosen: the page that published `command` runs it, or Create applies
/// `template` (File › New from Template).
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct MenuCommandRequested {
    pub command: Option<jutsu_app::system::app_menu::PageCommand>,
    pub template: Option<String>,
}

/// Files were opened from outside (Open with, a second launch, File › Open…): the window goes to Create, which takes
/// them with `opened_files_take`.
#[derive(Debug, Clone, Serialize, specta::Type, Event)]
pub struct FilesOpened {
    pub count: i32,
}

/// Sends `ConnectionChanged`, `PreferencesChanged`, `QueueChanged` and `CreateChanged` whenever their state changes, until the app quits. A new
/// output folder joins the media server's folders on the way.
pub(crate) fn forward(app: &AppHandle, environment: &Arc<AppEnvironment>, media: &MediaServer) {
    let mut connection = environment.connection.subscribe();
    let (handle, env) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move {
        while connection.changed().await.is_ok() {
            emit(
                &handle,
                &ConnectionChanged {
                    snapshot: env.connection_snapshot(),
                },
            );
        }
    });

    let mut queue = environment.queue.subscribe();
    let (handle, env) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move {
        while queue.changed().await.is_ok() {
            let active_count = i32::try_from(env.queue.active_count()).unwrap_or(i32::MAX);
            emit(&handle, &QueueChanged { active_count });
            if active_count == 0 {
                exit_if_hidden_for_jobs(&handle);
            }
        }
    });

    let mut preferences = environment.preferences.subscribe();
    let (handle, env, media) = (app.clone(), environment.clone(), media.clone());
    tauri::async_runtime::spawn(async move {
        while preferences.changed().await.is_ok() {
            for root in env.media_roots() {
                media.register_root(&root);
            }
            emit(
                &handle,
                &PreferencesChanged {
                    preferences: env.preferences.get(),
                },
            );
            // The plate's low-credit warning follows a setting.
            emit(
                &handle,
                &ConnectionChanged {
                    snapshot: env.connection_snapshot(),
                },
            );
        }
    });

    let mut create = environment.create.subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while create.changed().await.is_ok() {
            let revision = i32::try_from(*create.borrow_and_update() % (1 << 31)).unwrap_or_default();
            emit(&handle, &CreateChanged { revision });
        }
    });

    let mut images = environment.images.subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while images.changed().await.is_ok() {
            let revision = i32::try_from(*images.borrow_and_update() % (1 << 31)).unwrap_or_default();
            emit(&handle, &ImagesChanged { revision });
        }
    });

    let mut library = environment.library.subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while library.changed().await.is_ok() {
            let revision = i32::try_from(*library.borrow_and_update() % (1 << 31)).unwrap_or_default();
            emit(&handle, &LibraryChanged { revision });
        }
    });

    let mut on_device = environment.on_device.runs().subscribe();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while on_device.changed().await.is_ok() {
            let revision = i32::try_from(*on_device.borrow_and_update() % (1 << 31)).unwrap_or_default();
            emit(&handle, &OnDeviceRunsChanged { revision });
        }
    });

    let (mut runs, mut models) = environment.subtitles.subscribe();
    let handle = app.clone();
    let subtitles = environment.subtitles.clone();
    tauri::async_runtime::spawn(async move {
        let mut revision: i32 = 0;
        loop {
            tokio::select! {
                changed = runs.changed() => if changed.is_err() { return },
                changed = models.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // The model came or went: the languages a run needs no download for change with it.
                    models.borrow_and_update();
                    subtitles.runs().load_availability().await;
                }
            }
            revision = revision.wrapping_add(1) & i32::MAX;
            emit(&handle, &SubtitlesChanged { revision });
        }
    });
}

/// The window was closed while jobs ran, and the queue is now idle: quit, unless the window was opened again.
fn exit_if_hidden_for_jobs(app: &AppHandle) {
    let reopened = app
        .get_webview_window("main")
        .is_some_and(|window| window.is_visible().unwrap_or(true));
    if crate::take_hidden_for_jobs() && !reopened {
        tracing::info!("the queue is idle with the window closed; quitting");
        app.exit(0);
    }
}

fn emit<E: Event + Serialize + Clone>(app: &AppHandle, event: &E) {
    if let Err(error) = event.emit(app) {
        tracing::error!(%error, "can't send an event to the window");
    }
}
