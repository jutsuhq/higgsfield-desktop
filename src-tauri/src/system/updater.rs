//! Software Update (`SoftwareUpdater`) on tauri-plugin-updater. A build whose `plugins.updater` in `tauri.conf.json`
//! names no endpoint or public key, and every debug build, has no updater: Settings says so and nothing is fetched.
//! Otherwise the app checks once a day while Check automatically is on, and Check Now checks at once; a found update
//! is downloaded and installed when the person agrees, or downloaded at once and installed at quit while Download
//! and install automatically is on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jutsu_app::AppEnvironment;
use jutsu_app::settings::UpdatesState;
use jutsu_app::system::updates::{self, UpdateCheck, UpdatePreferences, UpdaterAvailability, UpdaterConfig};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};
use tauri_plugin_updater::{Update, UpdaterExt as _};

/// How often the scheduled check looks whether a day has passed since the last check.
const WAKE_INTERVAL: Duration = Duration::from_hours(1);

/// The update the last check found, and its download once made.
#[derive(Default)]
struct Pending {
    update: Option<Update>,
    downloaded: Option<Vec<u8>>,
}

#[derive(Default)]
struct UpdaterState {
    busy: AtomicBool,
    pending: Mutex<Pending>,
}

fn state(app: &AppHandle) -> Arc<UpdaterState> {
    if let Some(state) = app.try_state::<Arc<UpdaterState>>() {
        return state.inner().clone();
    }
    let state = Arc::new(UpdaterState::default());
    app.manage(state.clone());
    state
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// `plugins.updater` from `tauri.conf.json`.
fn config(app: &AppHandle) -> UpdaterConfig {
    app.config()
        .plugins
        .0
        .get("updater")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn availability(app: &AppHandle) -> UpdaterAvailability {
    UpdaterAvailability::new(cfg!(debug_assertions), &config(app))
}

/// What Settings › General shows of the updater.
pub(crate) fn updates_state(app: &AppHandle, environment: &AppEnvironment) -> UpdatesState {
    let preferences = UpdatePreferences::read(&environment.preferences);
    UpdatesState {
        availability: availability(app),
        preferences,
        last_check_text: updates::last_check_text(preferences.last_check, environment.utc_offset),
        is_busy: state(app).busy.load(Ordering::SeqCst),
    }
}

/// Checks for an update and keeps what it finds for `install`.
pub(crate) async fn check(app: &AppHandle, environment: &AppEnvironment) -> UpdateCheck {
    let availability = availability(app);
    if !availability.is_available() {
        return UpdateCheck::unavailable(availability);
    }
    let updater = state(app);
    if updater.busy.swap(true, Ordering::SeqCst) {
        return UpdateCheck::failed();
    }
    let found = match app.updater() {
        Ok(client) => client.check().await,
        Err(error) => Err(error),
    };
    updater.busy.store(false, Ordering::SeqCst);
    if let Err(error) = updates::record_check(&environment.preferences, now()) {
        tracing::warn!(%error, "can't store when updates were checked");
    }
    match found {
        Ok(Some(update)) => {
            let check = UpdateCheck::available(&update.version, &update.current_version);
            *updater.pending.lock().unwrap_or_else(PoisonError::into_inner) = Pending {
                update: Some(update),
                downloaded: None,
            };
            check
        }
        Ok(None) => UpdateCheck::up_to_date(env!("CARGO_PKG_VERSION")),
        Err(error) => {
            tracing::warn!(%error, "the update check failed");
            UpdateCheck::failed()
        }
    }
}

/// Downloads and installs the update the last check found, then starts the new version.
pub(crate) async fn install(app: &AppHandle) -> Result<(), String> {
    let update = state(app)
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .update
        .clone()
        .ok_or_else(|| "No update is waiting. Check for updates again.".to_owned())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    app.restart();
}

/// At quit: installs an update downloaded in the background. Nothing happens when none was.
pub(crate) fn install_downloaded(app: &AppHandle) {
    let pending = std::mem::take(&mut *state(app).pending.lock().unwrap_or_else(PoisonError::into_inner));
    if let (Some(update), Some(bytes)) = (pending.update, pending.downloaded)
        && let Err(error) = update.install(bytes)
    {
        tracing::error!(%error, "can't install the downloaded update");
    }
}

/// The scheduled check, while the build has an updater.
pub(crate) fn start(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    if !availability(app).is_available() {
        return;
    }
    let (app, environment) = (app.clone(), environment.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            let preferences = UpdatePreferences::read(&environment.preferences);
            if preferences.checks_automatically && preferences.is_check_due(now()) {
                checked_in_background(&app, &environment, preferences).await;
            }
            tokio::time::sleep(WAKE_INTERVAL).await;
        }
    });
}

async fn checked_in_background(
    app: &AppHandle,
    environment: &AppEnvironment,
    preferences: UpdatePreferences,
) {
    let check = check(app, environment).await;
    let Some(install_title) = check.install_title.clone() else {
        return;
    };
    let updater = state(app);
    let update = updater
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .update
        .clone();
    let Some(update) = update else {
        return;
    };
    if preferences.downloads() {
        match update.download(|_, _| {}, || {}).await {
            Ok(bytes) => {
                updater
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .downloaded = Some(bytes);
            }
            Err(error) => tracing::warn!(%error, "can't download the update"),
        }
        return;
    }
    offer_install(app, check, install_title);
}

/// Asks whether to install the update `check` found.
fn offer_install(app: &AppHandle, check: UpdateCheck, install_title: String) {
    let handle = app.clone();
    app.dialog()
        .message(check.message)
        .title(check.title)
        .buttons(MessageDialogButtons::OkCancelCustom(
            install_title,
            "Later".to_owned(),
        ))
        .show(move |installs| {
            if installs {
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = install(&handle).await {
                        tracing::error!(%error, "can't install the update");
                    }
                });
            }
        });
}

/// Whether a check runs now: Check for Updates… is dimmed meanwhile.
pub(crate) fn is_busy(app: &AppHandle) -> bool {
    state(app).busy.load(Ordering::SeqCst)
}

/// The menu's Check for Updates…: checks at once and says what it found, offering to install a newer version.
pub(crate) async fn check_from_menu(app: &AppHandle, environment: &AppEnvironment) {
    let check = check(app, environment).await;
    match check.install_title.clone() {
        Some(install_title) => offer_install(app, check, install_title),
        None => app
            .dialog()
            .message(check.message)
            .title(check.title)
            .show(|_| {}),
    }
}
