//! A video job as an agent's `jobs` sees it, what an agent may do to one with `job_action`, the pages `jobs` lists
//! them in, and the waits that follow them (`AgentJob+Job.swift`).

use std::future::Future;
use std::time::Duration;

use jutsu_agent::{AgentJob, Timestamp};
use jutsu_core::{AgentHold, AgentMoney, AgentMoneyUnit, Job, JobState, Mode, PauseReason, PixelSize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::budget;
use super::wire_id;
use crate::queue::format::{failure_text, rejection_title};
use crate::queue::row::reports_progress;

/// The names agents use for the modes, as `create_video` takes them.
pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "motion_transfer",
        Mode::Restyle => "restyle",
        Mode::ObjectSwap => "object_swap",
        Mode::Edit => "edit",
        Mode::Extend => "extend",
    }
}

/// The mode an agent names; `None` for a name that isn't one.
pub fn mode_named(name: &str) -> Option<Mode> {
    [
        Mode::MotionTransfer,
        Mode::Restyle,
        Mode::ObjectSwap,
        Mode::Edit,
        Mode::Extend,
    ]
    .into_iter()
    .find(|mode| mode_name(*mode) == name)
}

pub fn state_name(state: &JobState) -> &'static str {
    match state {
        JobState::Preparing => "preparing",
        JobState::Uploading => "uploading",
        JobState::WaitingForSlot => "waiting_for_slot",
        JobState::Queued => "queued",
        JobState::Processing => "processing",
        JobState::Downloading => "downloading",
        JobState::Paused(_) => "paused",
        JobState::Done => "done",
        JobState::Failed { .. } => "failed",
        JobState::Rejected => "rejected",
        JobState::Canceled => "canceled",
    }
}

pub fn pause_name(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::InsufficientFunds => "insufficient_funds",
        PauseReason::ConnectionNeedsAttention => "connection_needs_attention",
        PauseReason::PriceAboveCap => "price_above_cap",
        PauseReason::NotEnoughDiskSpace => "not_enough_disk_space",
        PauseReason::KeptFailing => "kept_failing",
        PauseReason::ByUser => "by_user",
        PauseReason::AgentLimit => "agent_limit",
    }
}

pub fn hold_name(hold: AgentHold) -> &'static str {
    match hold {
        AgentHold::UnknownPrice => "unknown_price",
        AgentHold::OverMaxCost => "over_max_cost",
        AgentHold::OverDailyBudget => "over_daily_budget",
        AgentHold::LedgerUnavailable => "ledger_unavailable",
        AgentHold::UnlimitedNotAllowed => "unlimited_not_allowed",
        AgentHold::AboveApprovedPrice => "above_approved_price",
    }
}

/// A file URL's path; `None` for any other URL.
pub fn file_path(url: &Url) -> Option<String> {
    url.to_file_path().ok().map(|path| path.display().to_string())
}

/// A video job as an agent's `jobs` sees it: stable `snake_case` names rather than the Queue's words, and whether only
/// the person can move it on.
pub fn agent_job(job: &Job) -> AgentJob {
    let mut wire = AgentJob::new(
        wire_id(job.id),
        state_name(&job.state),
        Some(Timestamp(job.created_at)),
    );
    wire.request_id = job.agent.as_ref().map(|agent| agent.request_id.clone());
    wire.client = job.agent.as_ref().and_then(|agent| agent.client.clone());
    wire.mode = Some(mode_name(job.spec.mode).to_owned());
    wire.model = Some(job.spec.model.raw_value().to_owned());
    wire.next = AgentJobRules::check_hint(job);
    wire.finished_at = job.finished_at.map(Timestamp);
    if let JobState::Paused(reason) = job.state {
        wire.pause_reason = Some(pause_name(reason).to_owned());
        wire.hold = if reason == PauseReason::AgentLimit {
            job.agent
                .as_ref()
                .and_then(|agent| agent.hold)
                .map(|hold| hold_name(hold).to_owned())
        } else {
            None
        };
        wire.needs_person = reason != PauseReason::ByUser;
    }
    if reports_progress(&job.state) {
        wire.progress = Some(job.progress.clamp(0.0, 1.0));
    }
    let price = if job.spec.uses_unlimited {
        Some(AgentMoney::new(0, AgentMoneyUnit::Credits))
    } else {
        AgentMoney::from_cost(job.price())
    };
    wire.price = price.map(budget::amount);
    wire.is_price_exact = job.agent_price_is_known();
    wire.charged = job.is_charged()
        || job.may_have_been_charged()
        || job.submit_attempt.is_some()
        || job.remote.is_some();
    wire.error = match &job.state {
        JobState::Failed { reason } => Some(failure_text(reason)),
        JobState::Rejected => Some(
            job.rejection
                .map_or("Rejected by content moderation", rejection_title)
                .to_owned(),
        ),
        _ => None,
    };
    if job.state == JobState::Done {
        wire.result = job.output_file.as_ref().and_then(file_path);
        if let Some(media) = &job.output_media {
            wire.width = Some(media.pixel_size.width);
            wire.height = Some(media.pixel_size.height);
            wire.duration = Some(media.duration);
            wire.frame_note = AgentJobRules::frame_note(
                job.spec.source.as_ref().map(|source| source.pixel_size),
                media.pixel_size,
            );
        }
    }
    wire
}

/// Finished, or waiting for someone: a `jobs` wait ends once every job it names is.
pub fn is_settled(state: &JobState) -> bool {
    state.is_terminal() || matches!(state, JobState::Paused(_))
}

/// The price is known as the spending guard reads it, or the job got past the guard, or it was held or finished: a
/// `create_video` wait ends once every job it queued is. A known price can still be held at submit, above `max_cost` or
/// the budgets; the agent sees both in the answer.
pub fn is_priced(job: &Job) -> bool {
    is_settled(&job.state)
        || job.agent_price_is_known()
        || job.submit_attempt.is_some()
        || job.remote.is_some()
}

/// What an agent may do to a job with `job_action`, and why not. Only jobs an agent queued; nothing that could pay
/// twice; nothing that resumes a job waiting for the person.
pub struct AgentJobRules;

impl AgentJobRules {
    pub fn cancel_refusal(job: &Job) -> Option<&'static str> {
        job.agent
            .is_none()
            .then_some("Only jobs an agent queued can be canceled by an agent.")
    }

    /// `file_exists`: whether a local input is still there; an agent's uploads are swept from the inbox a day after
    /// their job finished, and a retry without them would only fail once it runs.
    pub fn retry_refusal(job: &Job, file_exists: impl Fn(&Url) -> bool) -> Option<&'static str> {
        if job.agent.is_none() {
            return Some("Only jobs an agent queued can be retried by an agent.");
        }
        if job.may_have_been_charged() {
            return Some(
                "The job may already have been charged; only the person can decide to retry it in the Queue.",
            );
        }
        let mut inputs = job
            .spec
            .source
            .iter()
            .map(|source| &source.file_url)
            .chain(job.spec.references.iter().map(|image| &image.file_url));
        if inputs.any(|url| url.scheme() == "file" && !file_exists(url)) {
            return Some(
                "The job's input files are gone from Jutsu Studio, so a retry can't run. Send create_video again with \
                 the files and a new request_id.",
            );
        }
        None
    }

    pub fn run_next_refusal(job: &Job) -> Option<&'static str> {
        job.agent
            .is_none()
            .then_some("Only jobs an agent queued can be moved ahead by an agent.")
    }

    /// For a finished video: how to look at it with `frames`, against its own source when it has one, rather than
    /// grabbing frames by hand.
    pub fn check_hint(job: &Job) -> Option<String> {
        if job.state != JobState::Done || job.output_file.is_none() {
            return None;
        }
        let item = wire_id(job.id);
        Some(if job.spec.source.is_none() {
            format!(
                "To check it, call frames with item {item} and a few times; it writes the frames and a contact sheet."
            )
        } else {
            format!(
                "To check it against its source, call frames with item {item}, a few times and compare_source true; it \
                 writes both clips' frames side by side on a contact sheet."
            )
        })
    }

    /// Says so when a finished video's frame isn't its source's shape. Shapes within 2 % of each other are the same
    /// frame, as `PixelSize::aspect_label` names them.
    #[allow(clippy::cast_precision_loss, reason = "pixel sizes are far below 2^52")]
    pub fn frame_note(source: Option<PixelSize>, result: PixelSize) -> Option<String> {
        let source = source?;
        if !source.is_valid() || !result.is_valid() {
            return None;
        }
        let difference = (result.width as f64 / result.height as f64).ln()
            - (source.width as f64 / source.height as f64).ln();
        if difference.abs() < 0.02 {
            return None;
        }
        let shape = |size: PixelSize| {
            format!(
                "{}×{} ({})",
                size.width,
                size.height,
                size.aspect_label().unwrap_or_else(|| "?".to_owned())
            )
        };
        Some(format!(
            "The result is {}, not the source's frame, {}: the model chose its own. Scaling it to the source's frame \
             stretches it; check it with frames, then crop it (delivery_set) or make it again.",
            shape(result),
            shape(source)
        ))
    }

    /// Why a job sent again with a higher `max_cost` still waits: the hold it would get now, or `None` for the
    /// person's daily cap. Nothing changed, so the job stays held over its old `max_cost`, which the same request can
    /// lift later, once the budget or the cap has room.
    pub fn still_held_note(hold: Option<AgentHold>) -> &'static str {
        match hold {
            Some(AgentHold::OverMaxCost) => {
                "The price is still above max_cost. Send the request again with a max_cost of at least the price, or \
                 ask the person to confirm it in the Queue."
            }
            Some(AgentHold::OverDailyBudget) => {
                "The price is more than is left of today's agent budget, so the job stays held over its old max_cost. \
                 Send the request again once the budget has room (tomorrow, or after the person raises it in Settings \
                 › Agents), or ask the person to confirm it in the Queue."
            }
            None => {
                "The price would take today's spending over the person's daily cap, so the job stays held over its \
                 old max_cost. Send the request again once the cap has room, or ask the person to confirm it in the \
                 Queue."
            }
            Some(
                AgentHold::UnknownPrice
                | AgentHold::LedgerUnavailable
                | AgentHold::UnlimitedNotAllowed
                | AgentHold::AboveApprovedPrice,
            ) => {
                "The job can't be checked against the new max_cost now, so it stays held over its old max_cost. Send \
                 the request again in a moment, or ask the person to confirm it in the Queue."
            }
        }
    }
}

/// Where a page newest first ended: the date and ID of its last entry, as text an agent hands back
/// (`1729500000123_<UUID>`). Entries order by date, then by ID, so equal dates never repeat or drop one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentCursor {
    pub date: OffsetDateTime,
    pub id: Uuid,
}

impl AgentCursor {
    /// `None` for text no page handed out.
    pub fn parse(text: &str) -> Option<Self> {
        let (milliseconds, id) = text.split_once('_')?;
        let milliseconds: i64 = milliseconds.parse().ok()?;
        let id = Uuid::parse_str(id).ok()?;
        let date = OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000).ok()?;
        Some(Self { date, id })
    }

    pub fn text(&self) -> String {
        format!("{}_{}", milliseconds(self.date), wire_id(self.id))
    }

    /// Whether an entry made at `date` with `id` comes after this cursor, newest first.
    pub fn precedes(&self, date: OffsetDateTime, id: Uuid) -> bool {
        let (mine, theirs) = (milliseconds(self.date), milliseconds(date));
        if theirs == mine {
            id < self.id
        } else {
            theirs < mine
        }
    }
}

/// To the millisecond a cursor keeps, rounded down.
fn milliseconds(date: OffsetDateTime) -> i64 {
    let nanos = date.unix_timestamp_nanos().div_euclid(1_000_000);
    i64::try_from(nanos.clamp(-9_000_000_000_000_000, 9_000_000_000_000_000)).unwrap_or(0)
}

/// Newest first, then by ID, to the millisecond a cursor keeps.
pub fn newer_first(a: &(OffsetDateTime, Uuid), b: &(OffsetDateTime, Uuid)) -> std::cmp::Ordering {
    milliseconds(b.0)
        .cmp(&milliseconds(a.0))
        .then_with(|| b.1.cmp(&a.1))
}

/// A page of entries of any kind, newest first, after a cursor an earlier page handed out: the IDs of the page, and
/// the cursor of the next one when there are more.
pub fn page(
    mut entries: Vec<(OffsetDateTime, Uuid)>,
    limit: usize,
    cursor: Option<&str>,
) -> (Vec<Uuid>, Option<String>) {
    entries.sort_by(newer_first);
    let rest: Vec<(OffsetDateTime, Uuid)> = match cursor.and_then(AgentCursor::parse) {
        Some(position) => entries
            .into_iter()
            .filter(|(date, id)| position.precedes(*date, *id))
            .collect(),
        None => entries,
    };
    let taken = limit.max(1).min(rest.len());
    let next = if rest.len() > taken {
        rest.get(taken - 1)
            .map(|(date, id)| AgentCursor { date: *date, id: *id }.text())
    } else {
        None
    };
    (rest.into_iter().take(taken).map(|(_, id)| id).collect(), next)
}

/// The agents' jobs of a page: only jobs an agent queued.
pub fn job_page(jobs: &[Job], limit: usize, cursor: Option<&str>) -> (Vec<Job>, Option<String>) {
    let agents: Vec<&Job> = jobs.iter().filter(|job| job.agent.is_some()).collect();
    let (ids, next) = page(
        agents.iter().map(|job| (job.created_at, job.id)).collect(),
        limit,
        cursor,
    );
    let page = ids
        .iter()
        .filter_map(|id| agents.iter().find(|job| job.id == *id).map(|job| (*job).clone()))
        .collect();
    (page, next)
}

/// Waits up to `seconds` for `is_settled`, checking every `step`; `true` when the time ran out first. Waiting stops
/// nothing. The time is read from `now` rather than counted in checks, so sleeps that run long still end the wait on
/// time.
pub async fn agent_wait_with<S, SF, N, P, PF>(
    seconds: i64,
    step: Duration,
    mut is_settled: S,
    now: N,
    mut sleep: P,
) -> bool
where
    S: FnMut() -> SF,
    SF: Future<Output = bool>,
    N: Fn() -> Duration,
    P: FnMut(Duration) -> PF,
    PF: Future<Output = ()>,
{
    let step = step.max(Duration::from_millis(1));
    let deadline = now() + Duration::from_secs(u64::try_from(seconds.max(0)).unwrap_or(0));
    if is_settled().await {
        return false;
    }
    while now() < deadline {
        sleep(step.min(deadline.saturating_sub(now()))).await;
        if is_settled().await {
            return false;
        }
    }
    true
}

/// `agent_wait_with` on tokio's clock, checking every half second.
pub async fn agent_wait<S, SF>(seconds: i64, is_settled: S) -> bool
where
    S: FnMut() -> SF,
    SF: Future<Output = bool>,
{
    let start = tokio::time::Instant::now();
    agent_wait_with(
        seconds,
        Duration::from_millis(500),
        is_settled,
        || start.elapsed(),
        tokio::time::sleep,
    )
    .await
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
