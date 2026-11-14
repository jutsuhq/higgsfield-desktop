//! The agent server's window side: whether the window is open, and `show`, which brings it forward and asks the React
//! app to route there (`AgentShowRequested`).

use std::sync::Arc;

use jutsu_app::AppEnvironment;
use jutsu_app::agents::runtime::AgentRuntime;
use jutsu_app::agents::service::{AgentShowTarget, AgentWindow};
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::events::AgentShowRequested;

struct TauriWindow {
    app: AppHandle,
    environment: Arc<AppEnvironment>,
}

impl AgentWindow for TauriWindow {
    fn is_open(&self) -> bool {
        self.app
            .get_webview_window("main")
            .is_some_and(|window| window.is_visible().unwrap_or(false))
    }

    /// Settings › General › Keep in the menu bar, as the system side reads it.
    fn runs_in_tray(&self) -> bool {
        self.environment.preferences.get().runs_in_menu_bar()
    }

    fn show(&self, target: &AgentShowTarget) -> bool {
        if self.app.get_webview_window("main").is_none() {
            return false;
        }
        crate::bring_to_front(&self.app);
        let event = AgentShowRequested {
            route: target.route(),
        };
        if let Err(error) = event.emit(&self.app) {
            tracing::error!(%error, "can't ask the window to show a page");
            return false;
        }
        true
    }
}

/// Builds the agent server and starts listening; the runtime is kept as managed state.
pub(crate) fn start(app: &AppHandle, environment: Arc<AppEnvironment>) {
    let window = Arc::new(TauriWindow {
        app: app.clone(),
        environment: environment.clone(),
    });
    let runtime = Arc::new(AgentRuntime::new(
        environment,
        window,
        env!("CARGO_PKG_VERSION"),
        None,
    ));
    let started = runtime.clone();
    tauri::async_runtime::block_on(async move { started.start() });
    app.manage(runtime);
}
