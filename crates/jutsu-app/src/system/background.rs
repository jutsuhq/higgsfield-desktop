//! What the app does outside its window while jobs run (`BackgroundPolicy.swift`) and Settings › General › Run in
//! the system tray (`MenuBarMode.swift`): when closing the window keeps the app running, when it starts with no
//! window, what the taskbar badge says and what quitting asks.

use serde::Serialize;

use crate::settings::shows_tray_icon;

/// How long quitting waits for submits already sent to be answered (`JobEngine::stop`). Cut off, a submit without an
/// idempotency key can't be sent again, so the job fails as possibly started.
pub const SUBMIT_GRACE_SECONDS: u64 = 5;

/// The work outside the queue a quit or a closed window has to wait for: Generate… requests being sent or made, and
/// finished images being saved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImageWork {
    /// Requests being sent or followed, and images being saved.
    pub running: usize,
    /// Among them, submits whose answer hasn't come and images being saved: quitting waits for these.
    pub waiting: usize,
}

impl ImageWork {
    pub const NONE: Self = Self {
        running: 0,
        waiting: 0,
    };
}

/// Why the app is quitting. macOS reads it from the quit Apple event; here a quit from the session ending (log out,
/// restart, shut down) is `System`, everything else (Quit in the tray, the panel, a shortcut) is `User`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitReason {
    User,
    LogOut,
    Restart,
    ShutDown,
}

/// The confirmation shown on quit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct QuitPrompt {
    pub message: String,
    pub detail: String,
}

/// Closing the last window keeps the app alive while the queue holds a job, paused ones included, or work outside the
/// queue runs, so the tray and notifications keep working and nothing being paid for is cut off.
pub fn keeps_running(active_count: usize, images: ImageWork) -> bool {
    active_count > 0 || images.running > 0
}

/// The taskbar badge (`BackgroundPolicy.dockBadge`); `None` removes it.
pub fn taskbar_badge(active_count: usize) -> Option<String> {
    (active_count > 0).then(|| active_count.to_string())
}

/// The confirmation shown on quit; `None` quits right away. Log out, restart and shut down never wait for an answer:
/// blocking them would stall the system, and the jobs resume on next launch anyway.
///
/// `submitting_count` is the jobs among `active_count` whose submit is out (`LiveJobs::submitting_count`).
pub fn quit_prompt(
    active_count: usize,
    submitting_count: usize,
    images: ImageWork,
    reason: QuitReason,
) -> Option<QuitPrompt> {
    if reason != QuitReason::User || (active_count == 0 && images.running == 0) {
        return None;
    }
    if active_count == 0 {
        let message = if images.running == 1 {
            "1 image request is still running.".to_owned()
        } else {
            format!("{} image requests are still running.", images.running)
        };
        return Some(QuitPrompt {
            message,
            detail: image_detail(images),
        });
    }
    let jobs = jobs_prompt(active_count, submitting_count);
    if images.running == 0 {
        return Some(jobs);
    }
    let also = if images.running == 1 {
        "1 image request is running too.".to_owned()
    } else {
        format!("{} image requests are running too.", images.running)
    };
    Some(QuitPrompt {
        message: jobs.message,
        detail: format!("{} {also} {}", jobs.detail, image_detail(images)),
    })
}

/// What quitting does to the requests outside the queue: the ones being sent or saved are waited for, and the ones
/// being made are followed again on next launch.
fn image_detail(images: ImageWork) -> String {
    let resumes = "Images still being made are followed again on next launch.";
    if images.waiting == 0 {
        return resumes.to_owned();
    }
    format!("Quitting waits up to {SUBMIT_GRACE_SECONDS} seconds for the ones being sent or saved. {resumes}")
}

fn jobs_prompt(active_count: usize, submitting_count: usize) -> QuitPrompt {
    let message = if active_count == 1 {
        "1 job is still running.".to_owned()
    } else {
        format!("{active_count} jobs are still running.")
    };
    let resumes = if active_count == 1 {
        "It resumes on next launch."
    } else {
        "They resume on next launch."
    };
    if submitting_count == 0 {
        return QuitPrompt {
            message,
            detail: resumes.to_owned(),
        };
    }
    let sending = if active_count == 1 {
        format!(
            "It is being sent to Higgsfield right now, so quitting waits up to {SUBMIT_GRACE_SECONDS} seconds for \
             Higgsfield to answer."
        )
    } else {
        let them = if submitting_count == 1 {
            "1 of them is".to_owned()
        } else {
            format!("{submitting_count} of them are")
        };
        format!(
            "{them} being sent to Higgsfield right now, so quitting waits up to {SUBMIT_GRACE_SECONDS} seconds for \
             Higgsfield to answer."
        )
    };
    QuitPrompt {
        message,
        detail: format!("{sending} {resumes}"),
    }
}

/// What started the app, as far as its first window goes (`MenuBarMode.Launch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// The person asked for the app: the Start menu, a launcher, a file or a link.
    Person,
    /// The autostart entry, which passes `LOGIN_LAUNCH_ARGUMENT`.
    Login,
    /// The `jutsu` helper, because an agent's call found the app not running.
    Helper,
}

/// What the autostart entry passes, so a start at login can be told from one the person asked for. macOS reads it
/// from the open-application event Login Items mark; Windows and Linux have no such event.
pub const LOGIN_LAUNCH_ARGUMENT: &str = "--started-at-login";

/// The launch the app's arguments describe: the helper passes `HELPER_LAUNCH_ARGUMENT`, the autostart entry
/// `LOGIN_LAUNCH_ARGUMENT`. The helper's argument wins.
pub fn launch(arguments: &[String]) -> Launch {
    if arguments
        .iter()
        .any(|argument| argument == jutsu_agent::HELPER_LAUNCH_ARGUMENT)
    {
        Launch::Helper
    } else if arguments.iter().any(|argument| argument == LOGIN_LAUNCH_ARGUMENT) {
        Launch::Login
    } else {
        Launch::Person
    }
}

/// Whether the app starts with no window. The helper always starts it so: the agent works through the socket, and
/// `show` opens the window when the person is to see something. At login only in tray mode, which the person chose;
/// off, Open at login opens the window as before.
pub fn starts_hidden(launch: Launch, runs_in_tray: bool) -> bool {
    match launch {
        Launch::Person => false,
        Launch::Login => runs_in_tray,
        Launch::Helper => true,
    }
}

/// Closing the window (`MenuBarMode.keepsRunning`): in tray mode the app always stays; otherwise while work runs.
/// macOS also counts the windows a hidden start closes; here a hidden start never opens one.
pub fn keeps_running_in_tray(runs_in_tray: bool, active_count: usize, images: ImageWork) -> bool {
    runs_in_tray || keeps_running(active_count, images)
}

/// The window was closed while work ran: whether the app quits once the work ends. It stays running while the tray
/// icon shows, as the macOS app stays in the Dock; with no icon nothing would be left to bring the window back.
pub fn quits_when_work_ends(show_in_tray: bool, runs_in_tray: bool) -> bool {
    !shows_tray_icon(show_in_tray, runs_in_tray)
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
