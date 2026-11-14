//! What the system tray shows (`MenuBarPanel.swift`, `MenuBarContent.swift`, `DockMenu.swift`): the panel's header
//! with the connection and the agents, today's agent budget, what waits for the person, what runs, the last results;
//! the icon's label, count and progress; and the tray menu, which carries the panel's key actions for the desktops
//! that give a tray icon no click.

use jutsu_agent::{AgentBudgetUnit, AgentPage, AgentSheet, Budget};
use jutsu_core::{AgentMoney, AgentMoneyUnit, GenerationTimes, Job, JobState, PauseReason};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::agents::service::AgentShowTarget;
use crate::connection::{AccountPlate, ConnectionState};
use crate::queue::live::{newest_finished_first, oldest_first};
use crate::queue::{JobRowModel, RowAction, TimeEstimate, format};

/// More rows than these make the panel taller than a laptop screen.
pub const RUNNING_LIMIT: usize = 4;
pub const NEEDS_YOU_LIMIT: usize = 3;
pub const RECENT_LIMIT: usize = 3;

/// How many steps the icon's progress bar has: each step is a new image, so the icon is redrawn a dozen times over a
/// job, not at every progress report.
pub const ICON_STEPS: u32 = 12;

/// The app, its connection and the agents.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayHeader {
    /// "Pro · 12,500 credits", "API key · Committed today: $4.20", "Not connected".
    pub connection_line: String,
    /// "Low credits: 42 left".
    pub warning: Option<String>,
    pub agents: TrayAgents,
}

/// What the agents' toggle pauses or resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AgentsControl {
    /// Pauses the agents' jobs not yet sent (`JobEngine::pause_agent_jobs`).
    Pause,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgents {
    /// Settings › Agents › Allow agents.
    pub is_allowed: bool,
    /// "No agent connected", "claude-code connected", "2 agents connected"; "Agents off".
    pub status: String,
    /// `None` while the agents have no job to pause or resume.
    pub control: Option<AgentsControl>,
}

/// Today's agent budget, with a bar of what is spent and reserved.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayToday {
    /// "$4.20 spent".
    pub spent: String,
    /// "of $10.00"; `None` while no budget is set.
    pub limit: Option<String>,
    /// "$1.00 reserved"; `None` when nothing is.
    pub reserved: Option<String>,
    /// "$4.80 left", or that no budget is set.
    pub remaining: String,
    pub spent_fraction: f64,
    pub reserved_fraction: f64,
    /// Nothing is left: agents' paid requests wait for the person.
    pub is_used_up: bool,
    /// "Your daily cap: $12.00 of $50.00", the person's own cap, which counts every job.
    pub cap: Option<String>,
}

/// Where a Needs you row opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttentionTarget {
    /// The job's row in the Queue.
    Job { id: String },
    /// The item's sheet over the Library: `upscale_on_mac` or `subtitles`.
    Sheet { sheet: String, item: String },
}

/// What the panel, the tray menu or a notification opens in the main window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TrayOpen {
    /// The window as it was left.
    App,
    Create,
    Queue,
    Library,
    Settings,
    /// A Needs you row.
    Attention {
        target: AttentionTarget,
    },
    /// A job's row in the Queue: a running row.
    QueueJob {
        id: String,
    },
    /// A finished job's result in Library: a recent result.
    LibraryJob {
        id: String,
    },
}

impl TrayOpen {
    /// Where the main window routes; `None` leaves it on its page, as for an ID that doesn't parse.
    pub fn target(&self) -> Option<AgentShowTarget> {
        let id = |raw: &str| Uuid::parse_str(raw).ok();
        match self {
            Self::App => None,
            Self::Create => Some(AgentShowTarget::Page(AgentPage::Create)),
            Self::Queue => Some(AgentShowTarget::Page(AgentPage::Queue)),
            Self::Library => Some(AgentShowTarget::Page(AgentPage::Library)),
            Self::Settings => Some(AgentShowTarget::Page(AgentPage::Settings)),
            Self::Attention {
                target: AttentionTarget::Job { id: raw },
            }
            | Self::QueueJob { id: raw } => id(raw).map(AgentShowTarget::QueueJob),
            Self::Attention {
                target: AttentionTarget::Sheet { sheet, item },
            } => Some(AgentShowTarget::Sheet(id(item)?, AgentSheet::parse(sheet)?)),
            Self::LibraryJob { id: raw } => id(raw).map(AgentShowTarget::LibraryJob),
        }
    }
}

/// Something only the person can settle: a held job, or an agent's request for a model download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Attention {
    /// A job, or an item and its sheet: an item can wait in two sheets.
    pub id: String,
    pub title: String,
    pub reason: String,
    pub target: AttentionTarget,
    /// The row's tooltip: "Show in the Queue" for a job, "Open the item's sheet" for a sheet.
    pub hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum RunKind {
    Job,
    OnDevice,
    Subtitles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayCancel {
    pub is_enabled: bool,
    pub help: String,
}

/// A paid job or a run on this computer.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayRunning {
    pub id: String,
    pub kind: RunKind,
    pub title: String,
    /// "Generating · 40%", "Upscale 4× · 12%".
    pub detail: String,
    /// Reported progress, 0…1.
    pub progress: Option<f64>,
    /// A generating job against the typical time, when no progress is reported.
    pub estimate: Option<TimeEstimate>,
    /// `None` when it can't be canceled at all; disabled once the API processes a job.
    pub cancel: Option<TrayCancel>,
}

/// A finished result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayRecent {
    /// The job's ID, which Library opens.
    pub id: String,
    pub title: String,
    /// "3 min ago".
    pub detail: String,
    /// The result's file, for Show in File Manager.
    pub file: Option<String>,
}

/// A run on this computer, as the panel lists it: Enhance's or subtitles'.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub id: Uuid,
    pub kind: RunKind,
    pub title: String,
    pub detail: String,
    pub fraction: Option<f64>,
}

/// An agent's open request for a model download.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentAsk {
    pub item: Uuid,
    pub sheet: AgentSheet,
    pub title: String,
}

/// The agents' side, from the agent service.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentsInput {
    pub is_allowed: bool,
    /// The clients that sent requests lately, newest first (`AgentActivity::connected_clients`).
    pub connected_clients: Vec<String>,
    /// The open asks (`AgentActivity::open_asks`).
    pub asks: Vec<AgentAsk>,
    /// Today's budget as `capabilities` reports it; `None` while nothing is connected or before it is read.
    pub budget: Option<Budget>,
}

/// What the panel is built from.
#[derive(Debug, Clone)]
pub struct TrayInput<'a> {
    /// Every job the app knows, finished ones included for the recent results.
    pub jobs: Vec<&'a Job>,
    /// The runs on this computer waiting or running, in the order they run.
    pub runs: Vec<Run>,
    pub connection: &'a ConnectionState,
    /// What is committed today, for an API key's header; `None` when it can't be read.
    pub committed_today: Option<Decimal>,
    pub agents: AgentsInput,
    pub now: OffsetDateTime,
    /// For the time estimate of generating jobs.
    pub times: &'a GenerationTimes,
    pub low_credit_threshold: i64,
}

/// What the tray panel shows. The view hides each section that is empty.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayPanel {
    pub header: TrayHeader,
    pub today: Option<TrayToday>,
    pub needs_you: Vec<Attention>,
    /// "and 2 more", past `NEEDS_YOU_LIMIT`.
    pub needs_you_overflow: Option<String>,
    pub running: Vec<TrayRunning>,
    /// "and 2 more", past `RUNNING_LIMIT`.
    pub running_overflow: Option<String>,
    pub recent: Vec<TrayRecent>,
    /// Nothing waits, runs or finished lately: "Nothing running".
    pub is_idle: bool,
}

impl TrayPanel {
    pub fn new(input: &TrayInput<'_>) -> Self {
        let mut active: Vec<&Job> = input
            .jobs
            .iter()
            .copied()
            .filter(|job| job.state.is_active())
            .collect();
        active.sort_by(|lhs, rhs| oldest_first(lhs, rhs));
        let today = if input.agents.is_allowed {
            input.agents.budget.as_ref().and_then(today)
        } else {
            None
        };
        let plate = AccountPlate::new(input.connection, input.low_credit_threshold);
        // Today's cap line already counts every job, so the header doesn't repeat it.
        let spend = if today.as_ref().is_some_and(|today| today.cap.is_some()) {
            None
        } else {
            spend(input.connection, input.committed_today)
        };
        let header = TrayHeader {
            connection_line: connection_line(plate.as_ref(), spend.as_deref()),
            warning: plate.and_then(|plate| plate.warning),
            agents: agents(&input.agents, &active),
        };

        let rows: Vec<(&Job, JobRowModel)> = active
            .iter()
            .map(|job| {
                let typical = input.times.estimate(&job.spec);
                (*job, JobRowModel::new(job, input.now, typical.as_ref()))
            })
            .collect();
        let waiting: Vec<Attention> = input
            .agents
            .asks
            .iter()
            .map(ask_attention)
            .chain(
                rows.iter()
                    .filter(|(job, _)| needs_person(job))
                    .map(|(_, row)| job_attention(row)),
            )
            .collect();
        let work: Vec<TrayRunning> = rows
            .iter()
            .filter(|(job, _)| !needs_person(job))
            .map(|(_, row)| running_job(row))
            .chain(input.runs.iter().map(running_run))
            .collect();

        let mut done: Vec<&Job> = input
            .jobs
            .iter()
            .copied()
            .filter(|job| job.state == JobState::Done)
            .collect();
        done.sort_by(|lhs, rhs| newest_finished_first(lhs, rhs));
        let recent: Vec<TrayRecent> = done
            .into_iter()
            .take(RECENT_LIMIT)
            .map(|job| TrayRecent {
                id: job.id.to_string(),
                title: JobRowModel::new(job, input.now, None).title,
                detail: format::relative_time(job.finished_at.unwrap_or(job.created_at), input.now),
                file: job
                    .output_file
                    .as_ref()
                    .and_then(|url| url.to_file_path().ok())
                    .map(|path| path.to_string_lossy().into_owned()),
            })
            .collect();

        let needs_you: Vec<Attention> = waiting.iter().take(NEEDS_YOU_LIMIT).cloned().collect();
        let running: Vec<TrayRunning> = work.iter().take(RUNNING_LIMIT).cloned().collect();
        let is_idle = needs_you.is_empty() && running.is_empty() && recent.is_empty();
        Self {
            header,
            today,
            needs_you,
            needs_you_overflow: overflow(waiting.len(), NEEDS_YOU_LIMIT),
            running,
            running_overflow: overflow(work.len(), RUNNING_LIMIT),
            recent,
            is_idle,
        }
    }
}

/// The tray icon's tooltip: "Jutsu Studio, 2 active jobs".
pub fn accessibility_label(active_count: usize) -> String {
    match active_count {
        0 => "Jutsu Studio, no active jobs".to_owned(),
        1 => "Jutsu Studio, 1 active job".to_owned(),
        count => format!("Jutsu Studio, {count} active jobs"),
    }
}

/// The number next to the tray icon; `None` when the setting is off or nothing runs.
pub fn count_text(active_count: usize, is_shown: bool) -> Option<String> {
    (is_shown && active_count > 0).then(|| active_count.to_string())
}

/// How far the running work is, together, in `ICON_STEPS` steps; `None` when nothing runs. Work that reports no
/// progress yet counts as not started.
pub fn icon_progress(running: &[TrayRunning]) -> Option<f64> {
    if running.is_empty() {
        return None;
    }
    let total: f64 = running
        .iter()
        .map(|item| {
            item.progress
                .or_else(|| item.estimate.as_ref().map(|estimate| estimate.fraction))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0)
        })
        .sum();
    let steps = f64::from(ICON_STEPS);
    #[allow(clippy::cast_precision_loss, reason = "a handful of rows")]
    let count = running.len() as f64;
    Some((total / count * steps).floor() / steps)
}

/// How far all the work is, for the icon: every paid job not held for the person, and every run.
pub fn icon_progress_of(
    jobs: &[&Job],
    runs: &[Run],
    now: OffsetDateTime,
    times: &GenerationTimes,
) -> Option<f64> {
    let work: Vec<TrayRunning> = jobs
        .iter()
        .filter(|job| job.state.is_active() && !needs_person(job))
        .map(|job| {
            let typical = times.estimate(&job.spec);
            running_job(&JobRowModel::new(job, now, typical.as_ref()))
        })
        .chain(runs.iter().map(running_run))
        .collect();
    icon_progress(&work)
}

/// Held for the person: every pause but their own Pause all.
pub fn needs_person(job: &Job) -> bool {
    matches!(job.state, JobState::Paused(reason) if reason != PauseReason::ByUser)
}

/// "Pro · 12,500 credits", "API key · Committed today: $4.20", "Not connected".
fn connection_line(plate: Option<&AccountPlate>, spend: Option<&str>) -> String {
    let Some(plate) = plate else {
        return "Not connected".to_owned();
    };
    if plate.is_api_key {
        return spend.map_or_else(
            || plate.badge.clone(),
            |spend| format!("{} · {spend}", plate.badge),
        );
    }
    format!("{} · {} {}", plate.badge, plate.amount, plate.unit)
}

/// "Committed today: $12.40" with an API key, whose balance can't be read.
fn spend(connection: &ConnectionState, committed: Option<Decimal>) -> Option<String> {
    match connection {
        ConnectionState::ApiKey { .. } => {
            committed.map(|committed| format!("Committed today: {}", format::dollars(committed)))
        }
        ConnectionState::None | ConnectionState::Account(_) => None,
    }
}

fn agents(input: &AgentsInput, active: &[&Job]) -> TrayAgents {
    if !input.is_allowed {
        return TrayAgents {
            is_allowed: false,
            status: "Agents off".to_owned(),
            control: None,
        };
    }
    let status = match input.connected_clients.as_slice() {
        [] => "No agent connected".to_owned(),
        [client] => format!("{client} connected"),
        clients => format!("{} agents connected", clients.len()),
    };
    let agent_jobs = active.iter().filter(|job| job.agent.is_some());
    let control = if agent_jobs
        .clone()
        .any(|job| job.state.is_pausable_by_user() && job.submit_attempt.is_none())
    {
        Some(AgentsControl::Pause)
    } else if agent_jobs
        .into_iter()
        .any(|job| job.state == JobState::Paused(PauseReason::ByUser))
    {
        Some(AgentsControl::Resume)
    } else {
        None
    };
    TrayAgents {
        is_allowed: true,
        status,
        control,
    }
}

fn money(amount: i64, unit: AgentBudgetUnit) -> String {
    let unit = match unit {
        AgentBudgetUnit::UsdCents => AgentMoneyUnit::UsdCents,
        AgentBudgetUnit::Credits => AgentMoneyUnit::Credits,
    };
    crate::queue::row::money_text(AgentMoney { unit, amount })
}

fn today(budget: &Budget) -> Option<TrayToday> {
    if budget.daily_limit <= 0 && budget.daily_cap.is_none() {
        return None;
    }
    let money = |amount: i64| money(amount, budget.unit);
    #[allow(clippy::cast_precision_loss, reason = "budgets are far below 2^53")]
    let (limit, spent, reserved) = (
        budget.daily_limit.max(1) as f64,
        budget.spent_today as f64,
        budget.reserved_today as f64,
    );
    let has_limit = budget.daily_limit > 0;
    Some(TrayToday {
        spent: format!("{} spent", money(budget.spent_today)),
        limit: has_limit.then(|| format!("of {}", money(budget.daily_limit))),
        reserved: (budget.reserved_today > 0).then(|| format!("{} reserved", money(budget.reserved_today))),
        remaining: if has_limit {
            format!("{} left", money(budget.remaining))
        } else {
            "No agent budget set".to_owned()
        },
        spent_fraction: (spent / limit).min(1.0),
        reserved_fraction: (reserved / limit).min((1.0 - spent / limit).max(0.0)),
        is_used_up: has_limit && budget.remaining == 0,
        cap: budget.daily_cap.map(|cap| {
            format!(
                "Your daily cap: {} of {}",
                money(budget.cap_taken_today.unwrap_or(0)),
                money(cap)
            )
        }),
    })
}

fn job_attention(row: &JobRowModel) -> Attention {
    Attention {
        id: row.id.clone(),
        title: row.title.clone(),
        reason: row
            .note
            .as_ref()
            .map(|note| note.text.clone())
            .or_else(|| row.status_hint.clone())
            .unwrap_or_else(|| row.status_label.clone()),
        target: AttentionTarget::Job { id: row.id.clone() },
        hint: "Show in the Queue".to_owned(),
    }
}

fn ask_attention(ask: &AgentAsk) -> Attention {
    let reason = match ask.sheet {
        AgentSheet::UpscaleOnMac => "An agent asks to download the upscale model",
        AgentSheet::Subtitles => "An agent asks to download a speech model",
    };
    Attention {
        id: format!("{}:{}", ask.item, ask.sheet.as_str()),
        title: ask.title.clone(),
        reason: reason.to_owned(),
        target: AttentionTarget::Sheet {
            sheet: ask.sheet.as_str().to_owned(),
            item: ask.item.to_string(),
        },
        hint: "Open the item's sheet".to_owned(),
    }
}

fn running_job(row: &JobRowModel) -> TrayRunning {
    let cancel = row.actions.iter().rev().find_map(|action| match action {
        RowAction::Cancel { is_enabled, help } => Some(TrayCancel {
            is_enabled: *is_enabled,
            help: help.clone(),
        }),
        _ => None,
    });
    let detail = row.progress_text.as_ref().map_or_else(
        || row.status_label.clone(),
        |progress| format!("{} · {progress}", row.status_label),
    );
    TrayRunning {
        id: row.id.clone(),
        kind: RunKind::Job,
        title: row.title.clone(),
        detail,
        progress: row.progress,
        estimate: if row.progress.is_none() {
            row.time_estimate.clone()
        } else {
            None
        },
        cancel,
    }
}

fn running_run(run: &Run) -> TrayRunning {
    TrayRunning {
        id: run.id.to_string(),
        kind: run.kind,
        title: run.title.clone(),
        detail: run.detail.clone(),
        progress: run.fraction,
        estimate: None,
        cancel: Some(TrayCancel {
            is_enabled: true,
            help: "Cancel".to_owned(),
        }),
    }
}

fn overflow(count: usize, limit: usize) -> Option<String> {
    (count > limit).then(|| format!("and {} more", count - limit))
}

/// What a tray menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayCommand {
    Open,
    Queue,
    Library,
    PauseAll,
    ResumeAll,
    Settings,
    Quit,
}

impl TrayCommand {
    pub const ALL: [Self; 7] = [
        Self::Open,
        Self::Queue,
        Self::Library,
        Self::PauseAll,
        Self::ResumeAll,
        Self::Settings,
        Self::Quit,
    ];

    /// The menu item's identifier.
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "tray.open",
            Self::Queue => "tray.queue",
            Self::Library => "tray.library",
            Self::PauseAll => "tray.pause-all",
            Self::ResumeAll => "tray.resume-all",
            Self::Settings => "tray.settings",
            Self::Quit => "tray.quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }

    /// What the item opens; `None` for the items that act without the window.
    pub fn opens(self) -> Option<TrayOpen> {
        match self {
            Self::Open => Some(TrayOpen::App),
            Self::Queue => Some(TrayOpen::Queue),
            Self::Library => Some(TrayOpen::Library),
            Self::Settings => Some(TrayOpen::Settings),
            Self::PauseAll | Self::ResumeAll | Self::Quit => None,
        }
    }
}

/// A row of the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayMenuItem {
    Item {
        command: TrayCommand,
        title: String,
        is_enabled: bool,
    },
    Separator,
    /// A section's name, shown disabled.
    Section(String),
    /// One of the panel's actions.
    Action {
        action: PanelAction,
        title: String,
        is_enabled: bool,
    },
    Submenu {
        title: String,
        items: Vec<TrayMenuItem>,
    },
}

/// What a panel row does, from the panel or from the menu that stands for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    Open(TrayOpen),
    /// Cancels the job or the run with this ID.
    Cancel(String),
    /// Shows the file in the file manager.
    Reveal(String),
    Agents(AgentsControl),
}

/// The panel as menu items, for the desktops that give a tray icon no click (Linux): what waits for the person, what
/// runs, the last results and the agents' toggle, each doing what its row does in the panel. Nothing when idle.
pub fn panel_menu(panel: &TrayPanel) -> Vec<TrayMenuItem> {
    let action = |action, title: String, is_enabled| TrayMenuItem::Action {
        action,
        title,
        is_enabled,
    };
    let mut items = Vec::new();
    if !panel.needs_you.is_empty() {
        items.push(TrayMenuItem::Section("Needs you".to_owned()));
        items.extend(panel.needs_you.iter().map(|item| {
            action(
                PanelAction::Open(TrayOpen::Attention {
                    target: item.target.clone(),
                }),
                format!("{}: {}", item.title, item.reason),
                true,
            )
        }));
        items.extend(panel.needs_you_overflow.clone().map(TrayMenuItem::Section));
        items.push(TrayMenuItem::Separator);
    }
    if !panel.running.is_empty() {
        items.push(TrayMenuItem::Section("Running".to_owned()));
        items.extend(panel.running.iter().map(|item| {
            let mut actions = Vec::new();
            if item.kind == RunKind::Job {
                actions.push(action(
                    PanelAction::Open(TrayOpen::QueueJob { id: item.id.clone() }),
                    "Show in the Queue".to_owned(),
                    true,
                ));
            }
            if let Some(cancel) = &item.cancel {
                actions.push(action(
                    PanelAction::Cancel(item.id.clone()),
                    "Cancel".to_owned(),
                    cancel.is_enabled,
                ));
            }
            TrayMenuItem::Submenu {
                title: format!("{} · {}", item.title, item.detail),
                items: actions,
            }
        }));
        items.extend(panel.running_overflow.clone().map(TrayMenuItem::Section));
        items.push(TrayMenuItem::Separator);
    }
    if !panel.recent.is_empty() {
        items.push(TrayMenuItem::Section("Recent".to_owned()));
        items.extend(panel.recent.iter().map(|item| {
            let mut actions = vec![action(
                PanelAction::Open(TrayOpen::LibraryJob { id: item.id.clone() }),
                "Show in Library".to_owned(),
                true,
            )];
            if let Some(file) = &item.file {
                actions.push(action(
                    PanelAction::Reveal(file.clone()),
                    "Show in File Manager".to_owned(),
                    true,
                ));
            }
            TrayMenuItem::Submenu {
                title: format!("{} · {}", item.title, item.detail),
                items: actions,
            }
        }));
        items.push(TrayMenuItem::Separator);
    }
    if let Some(control) = panel.header.agents.control {
        let title = match control {
            AgentsControl::Pause => "Pause the agents' jobs",
            AgentsControl::Resume => "Resume the agents' jobs",
        };
        items.push(action(PanelAction::Agents(control), title.to_owned(), true));
        items.push(TrayMenuItem::Separator);
    }
    items
}

/// The tray menu: the Dock menu's items (`DockMenu.swift`) and the panel's Quit, built as things stand.
pub fn tray_menu(can_pause_all: bool, can_resume_all: bool) -> Vec<TrayMenuItem> {
    let item = |command, title: &str, is_enabled| TrayMenuItem::Item {
        command,
        title: title.to_owned(),
        is_enabled,
    };
    vec![
        item(TrayCommand::Open, "Open Jutsu Studio", true),
        item(TrayCommand::Queue, "Queue", true),
        item(TrayCommand::Library, "Library", true),
        TrayMenuItem::Separator,
        item(TrayCommand::PauseAll, "Pause All", can_pause_all),
        item(TrayCommand::ResumeAll, "Resume All", can_resume_all),
        TrayMenuItem::Separator,
        item(TrayCommand::Settings, "Settings…", true),
        TrayMenuItem::Separator,
        item(TrayCommand::Quit, "Quit Jutsu Studio", true),
    ]
}

#[cfg(test)]
#[path = "tray_tests.rs"]
mod tests;
