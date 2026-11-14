//! What a Queue row shows for one job (`JobRowModel.swift`, `+Formatting`, `+Notes`), and the Queue menu's job
//! items (`QueueJobCommands`), computed at a given `now`.

use jutsu_core::{
    AgentHold, AgentMoney, AgentMoneyUnit, AgentOrigin, GenerationEstimate, Job, JobState, PauseReason,
    ProviderKind, RetryCause, RetryStatus, VideoModel,
};
use rust_decimal::Decimal;
use serde::Serialize;
use time::OffsetDateTime;

use super::format;
use crate::connection::mode_title;

/// Semantic colors; the view maps them to the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ColorRole {
    Accent,
    Info,
    Warning,
    Danger,
    Faint,
}

/// A button of a row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowAction {
    /// Disabled once the API is processing the job; `help` is the tooltip either way.
    #[serde(rename_all = "camelCase")]
    Cancel {
        is_enabled: bool,
        help: String,
    },
    /// Runs a job that was never charged again.
    Retry,
    /// Fetches the paid result of a generation that finished but wasn't saved.
    RetryDownload,
    /// Polls a job past its deadline again.
    CheckAgain,
    /// Opens the inputs of a paid job in Create.
    GenerateAgain,
    Resume,
    /// Moves a job waiting for a slot ahead of the other waiters.
    RunNext,
    /// Resumes a job paused for its server price only after the user agrees to that price.
    ConfirmPrice,
    /// Rejected jobs: open the job's inputs in Create.
    Edit,
    /// The finished result in the file manager.
    ShowInFolder {
        path: String,
    },
    OpenInLibrary,
}

/// The steps a job goes through, shown as a stepper on active rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Prepare,
    Upload,
    Generate,
    Save,
}

impl Stage {
    pub const COUNT: f64 = 4.0;

    pub fn index(self) -> f64 {
        match self {
            Self::Prepare => 0.0,
            Self::Upload => 1.0,
            Self::Generate => 2.0,
            Self::Save => 3.0,
        }
    }

    pub fn of(state: &JobState) -> Option<Self> {
        match state {
            JobState::Preparing => Some(Self::Prepare),
            JobState::Uploading | JobState::WaitingForSlot => Some(Self::Upload),
            JobState::Queued | JobState::Processing => Some(Self::Generate),
            JobState::Downloading => Some(Self::Save),
            _ => None,
        }
    }
}

/// The line under the meta: why a job stopped or was not charged.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
pub struct RowNote {
    pub text: String,
    pub role: ColorRole,
}

impl RowNote {
    fn new(text: impl Into<String>, role: ColorRole) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }
}

/// How far a generating job is against the typical time of similar past jobs. An estimate, never a measured
/// percentage: the API reports no progress while it generates.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TimeEstimate {
    /// "usually ~2 min", or "taking longer than usual" once past it.
    pub label: String,
    /// Time since submit over the typical time, 0…1; stays at 1 once past it.
    pub fraction: f64,
    pub is_overdue: bool,
    /// Says where the estimate comes from, for a tooltip.
    pub help: String,
}

/// What a Queue row shows for one job.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobRowModel {
    pub id: String,
    /// "dance · Motion transfer".
    pub title: String,
    pub is_draft: bool,
    /// "Motion transfer · 720p · 00:00.0–00:12.0 · $8.17 · 3 min ago".
    pub meta: String,
    pub cost: String,
    /// "Estimate" or "Price from Higgsfield", for `cost`.
    pub price_source: String,
    pub is_charged: bool,
    pub status_label: String,
    pub status_role: ColorRole,
    /// What the status means, on hover.
    pub status_hint: Option<String>,
    /// The status dot pulses while the engine works on the job.
    pub is_working: bool,
    /// 0…1 for the states that report it.
    pub progress: Option<f64>,
    /// "42%".
    pub progress_text: Option<String>,
    pub note: Option<RowNote>,
    pub actions: Vec<RowAction>,
    /// The step the job is at; `None` while paused and once it has finished.
    pub stage: Option<Stage>,
    /// Time since the job was queued or last retried, "1:05"; active jobs only.
    pub elapsed: Option<String>,
    /// Time since the API accepted the job, "0:42"; on the Generate step only.
    pub generating: Option<String>,
    pub time_estimate: Option<TimeEstimate>,
    /// The prompt on one line.
    pub prompt: Option<String>,
    /// "Agent · claude-code" for a job an AI agent queued.
    pub origin: Option<String>,
    /// "Allowed up to $2.00".
    pub origin_detail: Option<String>,
}

impl JobRowModel {
    /// `now` for the relative time; `typical`, how long similar past jobs took to generate.
    #[allow(
        clippy::too_many_lines,
        reason = "one field after another, in the order of the Swift initializer"
    )]
    pub fn new(job: &Job, now: OffsetDateTime, typical: Option<&GenerationEstimate>) -> Self {
        let state = &job.state;
        let spec = &job.spec;
        let is_charged = !is_not_charged(job);
        let cost = cost_text(job, is_charged);
        let mode_or_action = spec.operation.as_ref().map_or_else(
            || mode_title(spec.mode).to_owned(),
            |operation| format::post_action_title(&operation.action),
        );
        let range = spec.source.as_ref().map(|source| source.range);
        let meta = [
            Some(mode_title(spec.mode).to_owned()),
            (spec.model != VideoModel::GENJUTSU).then(|| spec.model.title()),
            Some(format::resolution_title(spec.resolution, &spec.model)),
            Some(range.map_or_else(
                || format::limit(spec.duration()),
                |range| {
                    format!(
                        "{}–{}",
                        format::timecode(range.lower()),
                        format::timecode(range.upper())
                    )
                },
            )),
            (spec.variants > 1).then(|| format!("{} variants", spec.variants)),
            Some(cost.clone()),
            Some(format::relative_time(
                job.finished_at.unwrap_or_else(|| job.attempt_started_at()),
                now,
            )),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let (status_label, status_role) = status(state);
        let is_working = state.is_active() && !matches!(state, JobState::Paused(_));
        let (progress, progress_text) = if reports_progress(state) {
            let clamped = job.progress.clamp(0.0, 1.0);
            (Some(clamped), Some(format::percent(clamped)))
        } else {
            (None, None)
        };
        let mut note = note(
            state,
            job.provider_kind,
            is_charged,
            job.rejection,
            job.retrying.as_ref(),
            job.agent.as_ref().and_then(|agent| agent.hold),
        );
        if note.is_none()
            && is_working
            && let Some(retrying) = &job.retrying
        {
            note = Some(retry_note(retrying, now));
        }
        if note.is_none()
            && let Some(missing) = job.missing_variants.filter(|missing| *missing > 0)
        {
            note = Some(RowNote::new(
                format!("Higgsfield started fewer variants than asked for: {missing} missing"),
                ColorRole::Warning,
            ));
        }
        let step = Stage::of(state);
        let elapsed = state
            .is_active()
            .then(|| format::duration(seconds_between(job.attempt_started_at(), now)));
        let (generating, time_estimate) = match (step, job.submitted_at) {
            (Some(Stage::Generate), Some(submitted_at)) => {
                let since = seconds_between(submitted_at, now);
                (
                    Some(format::duration(since)),
                    typical.and_then(|typical| time_estimate(since, typical)),
                )
            }
            _ => (None, None),
        };
        let prompt = spec
            .prompt
            .lines()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_owned();
        Self {
            id: job.id.to_string(),
            title: format!("{} · {mode_or_action}", spec.name()),
            is_draft: spec.is_draft,
            meta,
            cost,
            price_source: price_source(job),
            is_charged,
            status_label: status_label.to_owned(),
            status_role,
            status_hint: (*state == JobState::WaitingForSlot).then(|| "Sent once a slot is free".to_owned()),
            is_working,
            progress,
            progress_text,
            note,
            actions: actions(job),
            stage: step,
            elapsed,
            generating,
            time_estimate,
            prompt: (!prompt.is_empty()).then_some(prompt),
            origin: job.agent.as_ref().map(origin_text),
            origin_detail: job
                .agent
                .as_ref()
                .map(|agent| format!("Allowed up to {}", money_text(agent.max_cost))),
        }
    }

    /// The finished result's video; `None` until there is one.
    pub fn file(&self) -> Option<&str> {
        self.actions.iter().find_map(|action| match action {
            RowAction::ShowInFolder { path } => Some(path.as_str()),
            _ => None,
        })
    }
}

fn seconds_between(from: OffsetDateTime, to: OffsetDateTime) -> f64 {
    (to - from).as_seconds_f64()
}

/// A job the API finished is charged even when its download then failed or was canceled.
pub fn is_not_charged(job: &Job) -> bool {
    job.state.is_terminal() && !job.is_charged()
}

/// The plan's unlimited generations, "Price unknown" for an operation no answer priced, else the price.
fn cost_text(job: &Job, charged: bool) -> String {
    if job.spec.uses_unlimited {
        return format::UNLIMITED.to_owned();
    }
    if job.price_is_unknown() && charged {
        return format::PRICE_UNKNOWN.to_owned();
    }
    format::format_cost(job.price(), charged)
}

/// Where the price comes from: the preliminary estimate, or the provider's quote.
pub fn price_source(job: &Job) -> String {
    if job.spec.uses_unlimited {
        return "Unlimited generations of the plan".to_owned();
    }
    if job.price_is_unknown() {
        return format::CHARGED_ON_COMPLETION.to_owned();
    }
    if job.server_price.is_none() && (job.provider_kind != ProviderKind::Account || job.cost.is_estimate()) {
        "Estimate".to_owned()
    } else {
        "Price from Higgsfield".to_owned()
    }
}

fn status(state: &JobState) -> (&'static str, ColorRole) {
    match state {
        JobState::Preparing => ("Preparing", ColorRole::Accent),
        JobState::Uploading => ("Uploading", ColorRole::Accent),
        JobState::WaitingForSlot => ("Waiting for slot", ColorRole::Info),
        JobState::Queued => ("Queued", ColorRole::Info),
        JobState::Processing => ("Processing", ColorRole::Accent),
        JobState::Downloading => ("Downloading", ColorRole::Accent),
        JobState::Paused(_) => ("Paused", ColorRole::Warning),
        JobState::Done => ("Done", ColorRole::Accent),
        JobState::Failed { .. } => ("Failed", ColorRole::Danger),
        JobState::Rejected => ("Rejected", ColorRole::Warning),
        JobState::Canceled => ("Canceled", ColorRole::Faint),
    }
}

/// The states whose `Job::progress` means something. Not `processing`: the API reports no progress while it
/// generates.
pub fn reports_progress(state: &JobState) -> bool {
    matches!(
        state,
        JobState::Preparing | JobState::Uploading | JobState::Downloading
    )
}

fn actions(job: &Job) -> Vec<RowAction> {
    match &job.state {
        JobState::Done => job
            .output_file
            .as_ref()
            .and_then(|url| url.to_file_path().ok())
            .map(|path| RowAction::ShowInFolder {
                path: path.to_string_lossy().into_owned(),
            })
            .into_iter()
            .chain([RowAction::OpenInLibrary])
            .collect(),
        JobState::Failed { .. } if job.may_still_finish() => vec![RowAction::CheckAgain],
        JobState::Failed { .. } | JobState::Canceled => {
            if job.has_paid_output() {
                vec![RowAction::RetryDownload, RowAction::GenerateAgain]
            } else {
                vec![RowAction::Retry]
            }
        }
        JobState::Rejected => vec![RowAction::Edit],
        JobState::Paused(PauseReason::PriceAboveCap | PauseReason::AgentLimit) => {
            vec![RowAction::ConfirmPrice, cancel(&job.state)]
        }
        JobState::Paused(_) => vec![RowAction::Resume, cancel(&job.state)],
        JobState::WaitingForSlot if job.submit_attempt.is_none() => {
            vec![RowAction::RunNext, cancel(&job.state)]
        }
        _ => vec![cancel(&job.state)],
    }
}

fn cancel(state: &JobState) -> RowAction {
    let (is_enabled, help) =
        if *state == JobState::Downloading || *state == JobState::Paused(PauseReason::NotEnoughDiskSpace) {
            (true, "Stop download")
        } else if state.is_cancellable() {
            (true, "Cancel job")
        } else {
            (false, "Can't cancel while processing")
        };
    RowAction::Cancel {
        is_enabled,
        help: help.to_owned(),
    }
}

/// `None` for a typical time of zero, which no bar can be filled against.
pub fn time_estimate(elapsed: f64, typical: &GenerationEstimate) -> Option<TimeEstimate> {
    let total = typical.total();
    if total <= 0.0 {
        return None;
    }
    let is_overdue = elapsed > total;
    Some(TimeEstimate {
        label: if is_overdue {
            "taking longer than usual".to_owned()
        } else {
            format!("usually {}", format::approximate(total))
        },
        fraction: (elapsed / total).clamp(0.0, 1.0),
        is_overdue,
        help: format!("Estimate from {} past jobs", typical.sample_count),
    })
}

/// "Agent · claude-code", or "Agent" when the client gave no name.
pub(crate) fn origin_text(agent: &AgentOrigin) -> String {
    match agent.client.as_deref() {
        Some(client) if !client.is_empty() => format!("Agent · {client}"),
        _ => "Agent".to_owned(),
    }
}

/// "$2.00" or "40 cr".
pub(crate) fn money_text(money: AgentMoney) -> String {
    match money.unit {
        AgentMoneyUnit::UsdCents => format::dollars(Decimal::new(money.amount, 2)),
        AgentMoneyUnit::Credits => format::credits(Decimal::from(money.amount)),
    }
}

const NOT_CHARGED: &str = "not charged";

fn note(
    state: &JobState,
    provider_kind: ProviderKind,
    charged: bool,
    rejection: Option<jutsu_core::Rejection>,
    retrying: Option<&RetryStatus>,
    agent_hold: Option<AgentHold>,
) -> Option<RowNote> {
    match state {
        JobState::Failed { reason } => Some(RowNote::new(
            format!(
                "{} · {}",
                format::failure_text(reason),
                if charged { "charged" } else { NOT_CHARGED }
            ),
            ColorRole::Danger,
        )),
        JobState::Rejected => {
            let title = rejection.map_or("Rejected by content moderation", format::rejection_title);
            Some(RowNote::new(
                format!("{title} · {NOT_CHARGED}"),
                ColorRole::Warning,
            ))
        }
        JobState::Canceled => Some(RowNote::new(
            if charged {
                "Download canceled · charged"
            } else {
                "Not charged"
            },
            ColorRole::Faint,
        )),
        JobState::Paused(PauseReason::AgentLimit) => Some(agent_note(agent_hold)),
        JobState::Paused(reason) => Some(paused_note(*reason, provider_kind, retrying)),
        _ => None,
    }
}

fn paused_note(reason: PauseReason, provider_kind: ProviderKind, retrying: Option<&RetryStatus>) -> RowNote {
    let warning = |text: &str| RowNote::new(text, ColorRole::Warning);
    match reason {
        PauseReason::InsufficientFunds => warning(if provider_kind == ProviderKind::Account {
            "Out of plan credits · top up, then resume"
        } else {
            "Not enough API balance · top up, then resume"
        }),
        PauseReason::ConnectionNeedsAttention => {
            warning("Connection needs attention · sign in again, then resume")
        }
        PauseReason::PriceAboveCap => {
            warning("Price from Higgsfield goes over today's spending cap · confirm to resume")
        }
        PauseReason::NotEnoughDiskSpace => {
            warning("Not enough disk space to save the result · free up space, then resume")
        }
        PauseReason::AgentLimit => agent_note(None),
        PauseReason::ByUser => RowNote::new(
            "Paused by you · nothing is submitted until you resume",
            ColorRole::Faint,
        ),
        PauseReason::KeptFailing => match retrying {
            None => warning("Kept failing before it was sent · not charged, resume to try again"),
            Some(retrying) => RowNote::new(
                format!(
                    "Kept failing before it was sent ({}) · not charged, resume to try again",
                    retry_cause(retrying.cause)
                ),
                ColorRole::Warning,
            ),
        },
    }
}

/// Why the spending guard stopped an agent's job; only the person resumes it.
fn agent_note(hold: Option<AgentHold>) -> RowNote {
    RowNote::new(agent_note_text(hold), ColorRole::Warning)
}

/// The words of `agent_note`, which a held image request's row shares (`AgentImageQueue`).
pub(crate) fn agent_note_text(hold: Option<AgentHold>) -> &'static str {
    match hold {
        Some(AgentHold::UnknownPrice) => "An agent's job at a price not known yet · confirm to send it",
        Some(AgentHold::OverMaxCost) => "Costs more than the agent was allowed · confirm to send it",
        Some(AgentHold::OverDailyBudget) => "Over today's agent budget · confirm to send it",
        Some(AgentHold::LedgerUnavailable) => {
            "Agent spending couldn't be read · nothing was sent, confirm to send it"
        }
        Some(AgentHold::UnlimitedNotAllowed) => {
            "Would use your plan's unlimited generations, which agents may not use · confirm to send it"
        }
        Some(AgentHold::AboveApprovedPrice) => {
            "Costs more now than the price you approved · confirm to send it"
        }
        None => "Stopped by the agent spending limits · confirm to send it",
    }
}

/// "Retrying in 30 s · attempt 4 · last error: Higgsfield server error (HTTP 502)", while a failed step waits to
/// run again; "Retrying" once the wait is over and the step runs.
fn retry_note(retrying: &RetryStatus, now: OffsetDateTime) -> RowNote {
    let attempt = retrying.failures + 1;
    let cause = retry_cause(retrying.cause);
    let wait = seconds_between(now, retrying.next_attempt_at).ceil();
    let text = if wait > 0.0 {
        format!(
            "Retrying in {} · attempt {attempt} · last error: {cause}",
            format::spoken(wait)
        )
    } else {
        format!("Retrying · attempt {attempt} · last error: {cause}")
    };
    RowNote::new(text, ColorRole::Warning)
}

fn retry_cause(cause: RetryCause) -> String {
    match cause {
        RetryCause::Network => "no connection to Higgsfield".to_owned(),
        RetryCause::Server { status } if status >= 500 => {
            format!("Higgsfield server error (HTTP {status})")
        }
        RetryCause::Server { status } => format!("Higgsfield answered HTTP {status}"),
        RetryCause::ModelUnavailable => "the model is unavailable on Higgsfield".to_owned(),
        RetryCause::ConcurrencyLimit => "all of the account's slots on Higgsfield are busy".to_owned(),
        RetryCause::InsufficientFunds => "not enough credits on Higgsfield".to_owned(),
    }
}

/// A command on one job. A Queue row's buttons and the Queue menu run the same ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum JobCommand {
    Cancel,
    /// Sends a job that was never charged again, or downloads the paid result of one again.
    Retry,
    /// Polls a job past its deadline again.
    CheckAgain,
    Resume,
    RunNext,
}

/// What the Queue menu's job items (and the keyboard) do with the selected job, by the rules of its row's buttons.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueJobCommands {
    /// The row offers Cancel and the job can still be canceled.
    pub can_cancel: bool,
    /// What Retry runs; `None` when the row offers neither Retry, Retry download nor Check again.
    pub retry: Option<JobCommand>,
    /// The Retry item's title, named after the row's button it stands for.
    pub retry_title: String,
}

impl QueueJobCommands {
    pub fn new(row: &JobRowModel) -> Self {
        let can_cancel = row
            .actions
            .iter()
            .any(|action| matches!(action, RowAction::Cancel { is_enabled: true, .. }));
        let (retry, retry_title) = if row.actions.contains(&RowAction::RetryDownload) {
            (Some(JobCommand::Retry), "Retry Download")
        } else if row.actions.contains(&RowAction::CheckAgain) {
            (Some(JobCommand::CheckAgain), "Check Again")
        } else {
            (
                row.actions
                    .contains(&RowAction::Retry)
                    .then_some(JobCommand::Retry),
                "Retry Job",
            )
        };
        Self {
            can_cancel,
            retry,
            retry_title: retry_title.to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
