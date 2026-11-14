//! The microphone on Linux: WebKitGTK keeps `getUserMedia` off unless `enable-media-stream` is set, and denies a
//! permission request nothing answers. Speech recordings need both, for the microphone only.

use tauri::{AppHandle, Manager};
use webkit2gtk::glib::prelude::Cast as _;
use webkit2gtk::{
    PermissionRequestExt as _, SettingsExt as _, UserMediaPermissionRequest,
    UserMediaPermissionRequestExt as _, WebViewExt as _,
};

use super::window::MAIN;

/// Turns on media streams in the main window and grants its requests for the microphone; the camera stays denied.
pub(crate) fn allow_microphone(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let configured = window.with_webview(|webview| {
        let view = webview.inner();
        if let Some(settings) = view.settings() {
            settings.set_enable_media_stream(true);
        }
        view.connect_permission_request(|_, request| {
            let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() else {
                return false;
            };
            if media.is_for_audio_device() && !media.is_for_video_device() {
                request.allow();
            } else {
                request.deny();
            }
            true
        });
    });
    if let Err(error) = configured {
        tracing::error!(%error, "can't turn on the microphone in the window");
    }
}
