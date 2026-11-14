//! The taskbar button (`SystemIntegration`'s Dock badge): the work's progress, and the number of jobs in the queue —
//! a count where the taskbar shows one, a lime dot over the icon on Windows, whose taskbar shows a picture only.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use jutsu_app::AppEnvironment;
use jutsu_app::system::tray_status;
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager};
use time::OffsetDateTime;

use super::window::MAIN;

/// What the taskbar shows, packed: the badge count and the progress percent, or `u64::MAX` before the first call.
static SHOWN: AtomicU64 = AtomicU64::new(u64::MAX);

/// Shows the queue's progress and count on the main window's taskbar button, when they changed.
pub(crate) fn apply(app: &AppHandle, environment: &Arc<AppEnvironment>) {
    let status = tray_status(environment, OffsetDateTime::now_utc());
    let count = status.badge.as_ref().map_or(0, |_| status.active_count);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a percent"
    )]
    let percent = status
        .progress
        .map(|fraction| (fraction.clamp(0.0, 1.0) * 100.0).round() as u64);
    let packed = ((count as u64) << 8) | percent.unwrap_or(0xff);
    if SHOWN.swap(packed, Ordering::SeqCst) == packed {
        return;
    }
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let progress = ProgressBarState {
        status: Some(if percent.is_some() {
            ProgressBarStatus::Normal
        } else {
            ProgressBarStatus::None
        }),
        progress: percent,
    };
    if let Err(error) = window.set_progress_bar(progress) {
        tracing::debug!(%error, "the taskbar shows no progress here");
    }
    badge(&window, count);
}

#[cfg(windows)]
fn badge(window: &tauri::WebviewWindow, count: usize) {
    let dot = (count > 0).then(|| {
        const SIZE: u32 = 16;
        tauri::image::Image::new_owned(jutsu_app::system::icons::badge_dot(SIZE), SIZE, SIZE)
    });
    if let Err(error) = window.set_overlay_icon(dot) {
        tracing::debug!(%error, "can't set the taskbar overlay");
    }
}

#[cfg(not(windows))]
fn badge(window: &tauri::WebviewWindow, count: usize) {
    let count = (count > 0).then(|| i64::try_from(count).unwrap_or(i64::MAX));
    if let Err(error) = window.set_badge_count(count) {
        tracing::debug!(%error, "the taskbar shows no count here");
    }
}
