//! `MenuBarPanelTests`, without the agent activity's own bookkeeping (`AgentActivity`), which the agent service owns.

use std::str::FromStr;

use jutsu_agent::{AgentBudgetUnit, AgentSheet, Budget};
use jutsu_core::{
    AgentMoney, AgentMoneyUnit, AgentOrigin, Cost, FailureReason, GenerationTimes, Job, JobState, PauseReason,
};
use rust_decimal::Decimal;
use url::Url;
use uuid::Uuid;

use super::*;
use crate::connection::{AccountInfo, ConnectionState};
use crate::queue::fixture::{JobFixture, now};

fn panel_with(jobs: &[Job], connection: &ConnectionState, agents: AgentsInput, runs: Vec<Run>) -> TrayPanel {
    panel_spending(jobs, connection, agents, runs, None)
}

fn panel_spending(
    jobs: &[Job],
    connection: &ConnectionState,
    agents: AgentsInput,
    runs: Vec<Run>,
    committed_today: Option<Decimal>,
) -> TrayPanel {
    let times = GenerationTimes::default();
    TrayPanel::new(&TrayInput {
        jobs: jobs.iter().collect(),
        runs,
        connection,
        committed_today,
        agents,
        now: now(),
        times: &times,
        low_credit_threshold: 0,
    })
}

fn panel(jobs: &[Job]) -> TrayPanel {
    panel_with(jobs, &ConnectionState::None, AgentsInput::default(), Vec::new())
}

fn allowed() -> AgentsInput {
    AgentsInput {
        is_allowed: true,
        ..AgentsInput::default()
    }
}

fn job(state: JobState, created_minutes_ago: f64) -> Job {
    JobFixture {
        created_minutes_ago,
        ..JobFixture::default()
    }
    .job(state)
}

fn agent_job(state: JobState) -> Job {
    let mut job = job(state, 3.0);
    job.agent = Some(AgentOrigin::new(
        Some("claude-code".to_owned()),
        "r",
        AgentMoney::new(500, AgentMoneyUnit::UsdCents),
    ));
    job
}

fn run(kind: RunKind, fraction: f64) -> Run {
    Run {
        id: Uuid::new_v4(),
        kind,
        title: "clip".to_owned(),
        detail: "Working".to_owned(),
        fraction: Some(fraction),
    }
}

fn budget(unit: AgentBudgetUnit, limit: i64, spent: i64, reserved: i64, remaining: i64) -> Budget {
    Budget {
        unit,
        daily_limit: limit,
        spent_today: spent,
        reserved_today: reserved,
        remaining,
        unlimited_allowed: false,
        daily_cap: None,
        cap_taken_today: None,
    }
}

#[test]
fn status_item_label_and_count() {
    assert_eq!(accessibility_label(0), "Jutsu Studio, no active jobs");
    assert_eq!(accessibility_label(1), "Jutsu Studio, 1 active job");
    assert_eq!(accessibility_label(2), "Jutsu Studio, 2 active jobs");
    assert_eq!(count_text(3, true).as_deref(), Some("3"));
    assert_eq!(count_text(3, false), None);
    assert_eq!(count_text(0, true), None);
}

#[test]
fn an_idle_panel_has_only_its_header() {
    let idle = panel(&[]);
    assert!(idle.is_idle);
    assert_eq!(idle.header.connection_line, "Not connected");
    assert_eq!(idle.header.agents.status, "Agents off");
    assert_eq!(idle.today, None);
}

#[test]
fn an_api_key_shows_what_is_committed_today() {
    let mut uploading = job(JobState::Uploading, 3.0);
    uploading.progress = 0.4;
    uploading.cost = Cost::usd(Decimal::from_str("2.5").unwrap(), true);
    let connection = ConnectionState::ApiKey {
        key_id: "key".to_owned(),
    };
    let result = panel_spending(
        &[uploading],
        &connection,
        AgentsInput::default(),
        Vec::new(),
        Some(Decimal::from_str("12.5").unwrap()),
    );
    assert_eq!(result.header.connection_line, "API key · Committed today: $12.50");
    assert_eq!(
        result
            .running
            .iter()
            .map(|item| item.progress)
            .collect::<Vec<_>>(),
        [Some(0.4)]
    );
    assert!(result.running[0].cancel.is_some());
}

#[test]
fn the_header_leaves_todays_spend_to_the_cap_line() {
    let connection = ConnectionState::ApiKey {
        key_id: "key".to_owned(),
    };
    let mut budget = budget(AgentBudgetUnit::UsdCents, 1000, 0, 0, 1000);
    budget.daily_cap = Some(5000);
    budget.cap_taken_today = Some(1250);
    let result = panel_spending(
        &[],
        &connection,
        AgentsInput {
            budget: Some(budget),
            ..allowed()
        },
        Vec::new(),
        Some(Decimal::from_str("12.5").unwrap()),
    );
    assert!(result.today.as_ref().is_some_and(|today| today.cap.is_some()));
    assert_eq!(result.header.connection_line, "API key");
}

#[test]
fn an_account_shows_its_credits() {
    let connection = ConnectionState::Account(AccountInfo {
        plan: Some("Pro".to_owned()),
        credits: Some(12_500),
        ..AccountInfo::default()
    });
    let account = panel_with(&[], &connection, AgentsInput::default(), Vec::new());
    assert_eq!(account.header.connection_line, "Pro · 12,500 credits");
}

/// Every pause but the person's own Pause all waits for them; their own pause stays with the running work.
#[test]
fn held_jobs_need_the_person_and_open_their_row() {
    let funds = job(JobState::Paused(PauseReason::InsufficientFunds), 9.0);
    let mut held = job(JobState::Paused(PauseReason::AgentLimit), 8.0);
    held.agent = agent_job(JobState::Queued).agent;
    let paused = job(JobState::Paused(PauseReason::ByUser), 7.0);
    let queued = job(JobState::Queued, 6.0);
    let result = panel(&[queued.clone(), paused.clone(), held.clone(), funds.clone()]);
    assert_eq!(
        result
            .needs_you
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>(),
        [
            AttentionTarget::Job {
                id: funds.id.to_string()
            },
            AttentionTarget::Job {
                id: held.id.to_string()
            }
        ]
    );
    assert_eq!(
        result
            .running
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        [paused.id.to_string(), queued.id.to_string()]
    );
    assert_eq!(result.needs_you_overflow, None);
    assert_eq!(result.needs_you[0].hint, "Show in the Queue");
}

#[test]
fn runs_follow_the_paid_jobs_and_overflow() {
    let jobs: Vec<Job> = (0..3)
        .map(|index| job(JobState::Queued, f64::from(10 - index)))
        .collect();
    let runs = vec![
        run(RunKind::OnDevice, 0.5),
        run(RunKind::Subtitles, 0.5),
        run(RunKind::Subtitles, 0.5),
    ];
    let result = panel_with(&jobs, &ConnectionState::None, AgentsInput::default(), runs);
    assert_eq!(result.running.len(), RUNNING_LIMIT);
    assert_eq!(
        result.running.iter().map(|item| item.kind).collect::<Vec<_>>(),
        [RunKind::Job, RunKind::Job, RunKind::Job, RunKind::OnDevice]
    );
    assert_eq!(result.running_overflow.as_deref(), Some("and 2 more"));
    assert!(
        result
            .running
            .last()
            .and_then(|item| item.cancel.as_ref())
            .is_some_and(|cancel| cancel.is_enabled)
    );
}

/// An agent's request for a model download waits in Needs you and opens the item's sheet.
#[test]
fn an_agents_ask_opens_the_items_sheet() {
    let item = Uuid::new_v4();
    let asked = panel_with(
        &[],
        &ConnectionState::None,
        AgentsInput {
            asks: vec![
                AgentAsk {
                    item,
                    sheet: AgentSheet::UpscaleOnMac,
                    title: "rooftop".to_owned(),
                },
                AgentAsk {
                    item,
                    sheet: AgentSheet::Subtitles,
                    title: "rooftop".to_owned(),
                },
            ],
            ..allowed()
        },
        Vec::new(),
    );
    assert_eq!(
        asked.needs_you[0].target,
        AttentionTarget::Sheet {
            sheet: "upscale_on_mac".to_owned(),
            item: item.to_string()
        }
    );
    assert_eq!(asked.needs_you[0].title, "rooftop");
    assert_eq!(
        asked.needs_you[0].reason,
        "An agent asks to download the upscale model"
    );
    assert_eq!(
        asked.needs_you[1].reason,
        "An agent asks to download a speech model"
    );
    assert_ne!(asked.needs_you[0].id, asked.needs_you[1].id);
    assert_eq!(asked.needs_you[0].hint, "Open the item's sheet");
}

#[test]
fn the_header_says_which_agents_are_connected() {
    let status = |clients: &[&str]| {
        panel_with(
            &[],
            &ConnectionState::None,
            AgentsInput {
                connected_clients: clients.iter().map(|client| (*client).to_owned()).collect(),
                ..allowed()
            },
            Vec::new(),
        )
        .header
        .agents
        .status
    };
    assert_eq!(status(&[]), "No agent connected");
    assert_eq!(status(&["claude-code"]), "claude-code connected");
    assert_eq!(status(&["claude-code", "codex"]), "2 agents connected");
}

/// Pause shows while an agent's job can still be stopped before its submit, Resume once the agents' jobs are paused.
#[test]
fn pause_and_resume_agents_follow_the_agents_jobs() {
    let control = |jobs: &[Job]| {
        panel_with(jobs, &ConnectionState::None, allowed(), Vec::new())
            .header
            .agents
            .control
    };
    assert_eq!(control(&[job(JobState::Queued, 3.0)]), None);
    assert_eq!(
        control(&[agent_job(JobState::WaitingForSlot)]),
        Some(AgentsControl::Pause)
    );
    assert_eq!(
        control(&[agent_job(JobState::Paused(PauseReason::ByUser))]),
        Some(AgentsControl::Resume)
    );
    assert_eq!(
        control(&[agent_job(JobState::Paused(PauseReason::AgentLimit))]),
        None
    );
}

#[test]
fn todays_budget_fills_its_bar() {
    let mut spending = budget(AgentBudgetUnit::UsdCents, 1000, 420, 100, 480);
    spending.daily_cap = Some(5000);
    spending.cap_taken_today = Some(1200);
    let with = |budget: Budget, is_allowed: bool| {
        panel_with(
            &[],
            &ConnectionState::None,
            AgentsInput {
                is_allowed,
                budget: Some(budget),
                ..AgentsInput::default()
            },
            Vec::new(),
        )
        .today
    };
    let today = with(spending.clone(), true).unwrap();
    assert_eq!(today.spent, "$4.20 spent");
    assert_eq!(today.reserved.as_deref(), Some("$1.00 reserved"));
    assert_eq!(today.limit.as_deref(), Some("of $10.00"));
    assert_eq!(today.remaining, "$4.80 left");
    assert!((today.spent_fraction - 0.42).abs() < 1e-9);
    assert!((today.reserved_fraction - 0.1).abs() < 1e-9);
    assert!(!today.is_used_up);
    assert_eq!(today.cap.as_deref(), Some("Your daily cap: $12.00 of $50.00"));
    assert_eq!(with(spending, false), None);

    assert_eq!(with(budget(AgentBudgetUnit::Credits, 0, 0, 0, 0), true), None);
    let full = with(budget(AgentBudgetUnit::Credits, 100, 80, 40, 0), true).unwrap();
    assert!(full.is_used_up);
    assert!(full.spent_fraction + full.reserved_fraction <= 1.0);
}

#[test]
fn recent_results_are_the_last_done_jobs() {
    let jobs: Vec<Job> = (1..=5)
        .map(|minutes| {
            let mut job = JobFixture {
                finished_minutes_ago: Some(f64::from(minutes)),
                ..JobFixture::default()
            }
            .job(JobState::Done);
            job.output_file = Some(Url::parse(&format!("file:///tmp/result-{minutes}.mp4")).unwrap());
            job
        })
        .collect();
    let failed = JobFixture {
        finished_minutes_ago: Some(0.5),
        ..JobFixture::default()
    }
    .job(JobState::Failed {
        reason: FailureReason::Message("x".to_owned()),
    });
    let mut all = jobs.clone();
    all.push(failed);
    let result = panel(&all);
    assert_eq!(
        result
            .recent
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        jobs.iter()
            .take(RECENT_LIMIT)
            .map(|job| job.id.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(result.recent[0].detail, "1 min ago");
    assert!(
        result.recent[0]
            .file
            .as_deref()
            .is_some_and(|file| file.ends_with("result-1.mp4"))
    );
    assert!(!result.is_idle);
}

#[test]
fn the_icons_progress_moves_in_steps() {
    assert_eq!(icon_progress(&[]), None);
    let runs = [run(RunKind::OnDevice, 0.5), run(RunKind::OnDevice, 0.2)];
    let held = job(JobState::Paused(PauseReason::InsufficientFunds), 3.0);
    let queued = job(JobState::Queued, 3.0);
    // (0 + 0.5 + 0.2) / 3 = 0.233…, down to 2/12; the held job doesn't count.
    let progress = icon_progress_of(&[&queued, &held], &runs, now(), &GenerationTimes::default()).unwrap();
    assert!((progress - 2.0 / 12.0).abs() < 0.0001);
}

#[test]
fn the_tray_menu_carries_the_panels_key_actions() {
    let titles = |items: &[TrayMenuItem]| {
        items
            .iter()
            .map(|item| match item {
                TrayMenuItem::Item {
                    title, is_enabled, ..
                } => format!("{title}{}", if *is_enabled { "" } else { " (off)" }),
                TrayMenuItem::Separator => "—".to_owned(),
                other => panic!("not in the plain menu: {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles(&tray_menu(true, false)),
        [
            "Open Jutsu Studio",
            "Queue",
            "Library",
            "—",
            "Pause All",
            "Resume All (off)",
            "—",
            "Settings…",
            "—",
            "Quit Jutsu Studio"
        ]
    );
    for command in TrayCommand::ALL {
        assert_eq!(TrayCommand::from_id(command.id()), Some(command));
    }
    assert_eq!(TrayCommand::from_id("other"), None);
}

#[test]
fn what_the_panel_and_the_menu_open() {
    let id = Uuid::new_v4();
    let item = Uuid::new_v4();
    assert_eq!(TrayOpen::App.target(), None);
    assert_eq!(
        TrayOpen::Settings.target(),
        Some(AgentShowTarget::Page(AgentPage::Settings))
    );
    assert_eq!(
        TrayOpen::QueueJob { id: id.to_string() }.target(),
        Some(AgentShowTarget::QueueJob(id))
    );
    assert_eq!(
        TrayOpen::LibraryJob { id: id.to_string() }.target(),
        Some(AgentShowTarget::LibraryJob(id))
    );
    assert_eq!(
        TrayOpen::Attention {
            target: AttentionTarget::Job { id: id.to_string() }
        }
        .target(),
        Some(AgentShowTarget::QueueJob(id))
    );
    assert_eq!(
        TrayOpen::Attention {
            target: AttentionTarget::Sheet {
                sheet: "upscale_on_mac".to_owned(),
                item: item.to_string()
            }
        }
        .target(),
        Some(AgentShowTarget::Sheet(item, AgentSheet::UpscaleOnMac))
    );
    assert_eq!(
        TrayOpen::QueueJob {
            id: "nope".to_owned()
        }
        .target(),
        None
    );
    assert_eq!(TrayCommand::Queue.opens(), Some(TrayOpen::Queue));
    assert_eq!(TrayCommand::Open.opens(), Some(TrayOpen::App));
    assert_eq!(TrayCommand::PauseAll.opens(), None);
    assert_eq!(TrayCommand::Quit.opens(), None);
}

#[test]
#[allow(clippy::too_many_lines, reason = "one panel with a row of each kind")]
fn the_menu_stands_for_the_panel_where_the_icon_takes_no_click() {
    let job = Uuid::new_v4().to_string();
    let panel = TrayPanel {
        header: TrayHeader {
            connection_line: "Not connected".to_owned(),
            warning: None,
            agents: TrayAgents {
                is_allowed: true,
                status: "No agent connected".to_owned(),
                control: Some(AgentsControl::Resume),
            },
        },
        today: None,
        needs_you: vec![Attention {
            id: job.clone(),
            title: "dance".to_owned(),
            reason: "Over the daily cap".to_owned(),
            target: AttentionTarget::Job { id: job.clone() },
            hint: "Show in the Queue".to_owned(),
        }],
        needs_you_overflow: Some("and 2 more".to_owned()),
        running: vec![TrayRunning {
            id: job.clone(),
            kind: RunKind::Job,
            title: "walk".to_owned(),
            detail: "Generating · 40%".to_owned(),
            progress: Some(0.4),
            estimate: None,
            cancel: Some(TrayCancel {
                is_enabled: false,
                help: "Higgsfield is processing it".to_owned(),
            }),
        }],
        running_overflow: None,
        recent: vec![TrayRecent {
            id: job.clone(),
            title: "jump".to_owned(),
            detail: "3 min ago".to_owned(),
            file: Some("/tmp/jump.mp4".to_owned()),
        }],
        is_idle: false,
    };
    let menu = panel_menu(&panel);
    assert_eq!(menu[0], TrayMenuItem::Section("Needs you".to_owned()));
    assert_eq!(
        menu[1],
        TrayMenuItem::Action {
            action: PanelAction::Open(TrayOpen::Attention {
                target: AttentionTarget::Job { id: job.clone() }
            }),
            title: "dance: Over the daily cap".to_owned(),
            is_enabled: true,
        }
    );
    assert_eq!(menu[2], TrayMenuItem::Section("and 2 more".to_owned()));
    let TrayMenuItem::Submenu { title, items } = &menu[5] else {
        panic!("{:?}", menu[5]);
    };
    assert_eq!(title, "walk · Generating · 40%");
    assert_eq!(
        items[1],
        TrayMenuItem::Action {
            action: PanelAction::Cancel(job.clone()),
            title: "Cancel".to_owned(),
            is_enabled: false,
        }
    );
    let TrayMenuItem::Submenu { items, .. } = &menu[8] else {
        panic!("{:?}", menu[8]);
    };
    assert_eq!(
        items
            .iter()
            .map(|item| match item {
                TrayMenuItem::Action { action, .. } => action.clone(),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        [
            PanelAction::Open(TrayOpen::LibraryJob { id: job }),
            PanelAction::Reveal("/tmp/jump.mp4".to_owned())
        ]
    );
    assert_eq!(
        menu[10],
        TrayMenuItem::Action {
            action: PanelAction::Agents(AgentsControl::Resume),
            title: "Resume the agents' jobs".to_owned(),
            is_enabled: true,
        }
    );
    assert_eq!(menu.len(), 12);

    let idle = TrayPanel {
        needs_you: Vec::new(),
        needs_you_overflow: None,
        running: Vec::new(),
        recent: Vec::new(),
        header: TrayHeader {
            agents: TrayAgents {
                control: None,
                ..panel.header.agents.clone()
            },
            ..panel.header.clone()
        },
        ..panel
    };
    assert_eq!(panel_menu(&idle), []);
}

/// Jobs made or finished at the same moment keep one order, whatever order the live jobs come in
/// (`LiveJobs.active` and `newestFinishedFirst`).
#[test]
fn ties_are_settled_by_the_id() {
    let mut first = job(JobState::Queued, 2.0);
    let mut second = first.clone();
    first.id = Uuid::from_u128(1);
    second.id = Uuid::from_u128(2);
    let order = |jobs: &[Job]| {
        panel(jobs)
            .running
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    };
    let expected = [first.id.to_string(), second.id.to_string()];
    assert_eq!(order(&[first.clone(), second.clone()]), expected);
    assert_eq!(order(&[second, first]), expected);

    let mut early = JobFixture {
        finished_minutes_ago: Some(1.0),
        ..JobFixture::default()
    }
    .job(JobState::Done);
    let mut late = early.clone();
    early.id = Uuid::from_u128(1);
    late.id = Uuid::from_u128(2);
    let recent = |jobs: &[Job]| {
        panel(jobs)
            .recent
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    };
    let newest = [late.id.to_string(), early.id.to_string()];
    assert_eq!(recent(&[early.clone(), late.clone()]), newest);
    assert_eq!(recent(&[late, early]), newest);
}
