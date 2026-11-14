//! Files opened from outside (`AppDelegate.application(_:open:)`): the arguments of the first launch and of a second
//! one, which the single-instance plugin hands over. They wait in `PendingOpens` until Create is on screen; the window
//! comes forward and goes to Create.

use std::path::Path;
use std::sync::Arc;

use jutsu_app::system::opened_files::{OpenRequest, PendingOpens, file_arguments};
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::events::FilesOpened;

/// The files waiting for Create.
pub(crate) fn pending(app: &AppHandle) -> Arc<PendingOpens> {
    if let Some(state) = app.try_state::<Arc<PendingOpens>>() {
        return state.inner().clone();
    }
    let state = Arc::new(PendingOpens::default());
    app.manage(state.clone());
    state
}

/// The files among a launch's `arguments`, run in `directory`; nothing happens without one.
pub(crate) fn open_arguments(app: &AppHandle, arguments: &[String], directory: &Path) {
    let files = file_arguments(arguments, directory);
    if !files.is_empty() {
        open(app, OpenRequest::new(files));
    }
}

/// Queues `request` for Create, brings the window forward and sends it to Create.
pub(crate) fn open(app: &AppHandle, request: OpenRequest) {
    if request.is_empty() {
        return;
    }
    let count = request.images.len() + request.unsupported.len() + usize::from(request.video.is_some());
    pending(app).add(request);
    crate::bring_to_front(app);
    let event = FilesOpened {
        count: i32::try_from(count).unwrap_or(i32::MAX),
    };
    if let Err(error) = event.emit(app) {
        tracing::error!(%error, "can't send the opened files to the window");
    }
}
