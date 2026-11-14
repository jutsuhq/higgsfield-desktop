//! The app outside its window (`JutsuStudio/System` with the rules of `Presentation`): the system tray's panel and
//! menu, job notifications, what closing the window and quitting do, the taskbar badge, and Software Update.
//! `src-tauri` applies them; nothing here knows Tauri.

pub mod about;
pub mod app_menu;
pub mod background;
pub mod icons;
pub mod notifications;
pub mod opened_files;
pub mod sidebar;
pub mod tray;
pub mod updates;
pub mod window_placement;

use std::sync::Mutex;

use time::OffsetDateTime;

use crate::AppEnvironment;
use crate::images::RunState;
use crate::preferences::Preferences;
use crate::settings::shows_tray_icon;
use background::{ImageWork, QuitPrompt, QuitReason};
use notifications::{JobNotification, JobTransitionTracker};
use tray::{AgentsInput, TrayInput, TrayMenuItem, TrayPanel};

/// The requests outside the queue a quit or a closed window waits for: Generate… runs being made, and the submits
/// whose answer hasn't come.
pub fn image_work(environment: &AppEnvironment) -> ImageWork {
    let generator = environment.images.generator();
    ImageWork {
        running: generator
            .runs()
            .iter()
            .filter(|run| run.state == RunState::Running)
            .count(),
        waiting: generator.sending_count(),
    }
}

/// What the tray icon, its menu and the taskbar show now.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayStatus {
    /// Settings › General › Show in the system tray, or tray mode, which forces it.
    pub shows_icon: bool,
    /// "Jutsu Studio, 2 active jobs", the icon's tooltip.
    pub tooltip: String,
    /// The count next to the icon, when Active job count is on.
    pub title: Option<String>,
    /// How far the work is, in steps; `None` while nothing runs.
    pub progress: Option<f64>,
    /// The taskbar badge: the jobs in the queue.
    pub badge: Option<String>,
    pub active_count: usize,
    pub menu: Vec<TrayMenuItem>,
}

/// The tray icon, its menu and the taskbar badge at `now`. Runs on this computer (Enhance, subtitles) count with the
/// jobs once they exist here.
pub fn tray_status(environment: &AppEnvironment, now: OffsetDateTime) -> TrayStatus {
    let preferences = environment.preferences.get();
    let shows_icon = shows_tray_icon(
        preferences
            .show_in_menu_bar
            .unwrap_or(Preferences::DEFAULT_SHOW_IN_MENU_BAR),
        preferences.runs_in_menu_bar(),
    );
    let shows_count = preferences
        .show_active_count
        .unwrap_or(Preferences::DEFAULT_SHOW_ACTIVE_COUNT);
    let runs = environment.on_device.tray_runs();
    environment.queue.with_live(|live| {
        let active_count = live.active_count();
        let jobs = live.all();
        TrayStatus {
            shows_icon,
            tooltip: tray::accessibility_label(active_count),
            title: tray::count_text(active_count, shows_count),
            progress: if active_count == 0 {
                None
            } else {
                tray::icon_progress_of(&jobs, &runs, now, &live.generation_times).or(Some(0.0))
            },
            badge: background::taskbar_badge(active_count),
            active_count,
            menu: tray::tray_menu(live.can_pause_all(), live.can_resume_all()),
        }
    })
}

/// The tray panel at `now`. `agents` comes from the agent service: whether agents are allowed, today's budget.
pub async fn tray_panel(environment: &AppEnvironment, agents: AgentsInput, now: OffsetDateTime) -> TrayPanel {
    let committed_today = environment.library.committed_today().await;
    let connection = environment.connection.state();
    let threshold = environment.preferences.get().low_credit_threshold();
    environment.queue.with_live(|live| {
        TrayPanel::new(&TrayInput {
            jobs: live.all(),
            runs: environment.on_device.tray_runs(),
            connection: &connection,
            committed_today,
            agents,
            now,
            times: &live.generation_times,
            low_credit_threshold: threshold,
        })
    })
}

/// What a quit asks while work runs; `None` quits at once.
pub fn quit_prompt(environment: &AppEnvironment, reason: QuitReason) -> Option<QuitPrompt> {
    let (active, submitting) = environment
        .queue
        .with_live(|live| (live.active_count(), live.submitting_count()));
    background::quit_prompt(active, submitting, image_work(environment), reason)
}

/// Whether closing the main window keeps the app running (`AppDelegate.applicationShouldTerminateAfterLastWindowClosed`).
pub fn keeps_running_without_window(environment: &AppEnvironment) -> bool {
    background::keeps_running_in_tray(
        environment.preferences.get().runs_in_menu_bar(),
        environment.queue.active_count(),
        image_work(environment),
    )
}

/// What a notification's Retry runs on job `id`: the Retry, Retry Download or Check Again its Queue row offers now.
pub fn retry_command(environment: &AppEnvironment, id: uuid::Uuid) -> Option<crate::queue::JobCommand> {
    environment.queue.with_live(|live| {
        let job = live.job(id)?;
        let row = crate::queue::JobRowModel::new(job, job.created_at, None);
        crate::queue::QueueJobCommands::new(&row).retry
    })
}

/// Posts each job's change once (`SystemIntegration.update`). Jobs that finished before it was made never notify.
#[derive(Debug)]
pub struct JobNotifier {
    tracker: Mutex<JobTransitionTracker>,
}

impl JobNotifier {
    pub fn new(since: OffsetDateTime) -> Self {
        Self {
            tracker: Mutex::new(JobTransitionTracker::new(since)),
        }
    }

    /// The notifications for what changed since the last call, as Settings › Notifications allows.
    pub fn changes(&self, environment: &AppEnvironment) -> Vec<JobNotification> {
        let preferences = environment.preferences.get().notification_preferences();
        let mut tracker = self
            .tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        environment
            .queue
            .with_live(|live| tracker.update(live.all(), preferences))
    }
}
