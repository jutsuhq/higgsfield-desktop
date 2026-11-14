//! The About window (`AboutView`'s `Window`): one of its own, 340 wide, not resizable, centered; opening it again
//! brings it forward.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The About window's label.
pub(crate) const LABEL: &str = "about";

pub(crate) fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // The page fits the window's height to its content once it has drawn.
    let made = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("about".into()))
        .title("About Jutsu Studio")
        .inner_size(340.0, 480.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .background_color(tauri::window::Color(6, 6, 7, 255))
        .build();
    if let Err(error) = made {
        tracing::error!(%error, "can't open the About window");
    }
}
