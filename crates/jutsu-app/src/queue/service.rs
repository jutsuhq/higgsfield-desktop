//! The Queue's state and actions (`AppEnvironment+Queue.swift`, `QueueView.swift`, `DuplicateJob.swift`): the live
//! jobs fed from the engine's events, the snapshot the screen shows, and the commands its buttons run. A failed
//! command shows its message as the connection alert, as the macOS app does.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jutsu_core::{
    Budget, BudgetDecision, Cost, Job, JobSpec, PauseReason, PortError, ProviderKind, SpendCap,
};
use jutsu_engine::{EngineError, EngineEvent, EnqueueOptions, JobEngine};
use jutsu_store::Store;
use rust_decimal::Decimal;
use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use super::batch_cancel::BatchCancel;
use super::format;
use super::live::{LiveJobs, QueueEntry, QueueProblems};
use super::row::{JobCommand, JobRowModel, QueueJobCommands, RowAction};
use crate::connection::{ConnectionStore, UserMessage};
use crate::preferences::PreferencesStore;
use crate::provider::ProviderHandle;

/// How many finished entries the Queue shows.
pub const FINISHED_LIMIT: usize = 8;
/// A job finished less than this many seconds ago glows.
const FRESH_SECONDS: f64 = 10.0;

/// One row of the Queue with what its buttons need.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueRow {
    pub model: JobRowModel,
    pub commands: QueueJobCommands,
    /// `request_id=… correlation_id=… app=… build=… os=…` for a failed or rejected job the API knows.
    pub support_ids: Option<String>,
    pub preview: JobPreview,
    /// The question Review price asks; set when the row offers it.
    pub price_confirmation: Option<QueueConfirmation>,
    /// Finished moments ago.
    pub is_fresh: bool,
    /// The whole prompt, the one-line prompt's tooltip; `None` without one.
    pub prompt_hint: Option<String>,
}

/// What goes in and what comes out of a job (`JobPreview`): a frame of the source, "+", up to three references, an
/// arrow, then the result. Files are paths for the media server; `None` where the file isn't on this computer (a job
/// from the account's history names its inputs by their links), which leaves a plain well.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobPreview {
    /// The job has a source video; a clip made without one shows no source tile.
    pub has_source: bool,
    pub source: Option<String>,
    /// At most three.
    pub references: Vec<Option<String>>,
    /// How many references `references` leaves out ("+2").
    pub more_references: i32,
    pub result: ResultTile,
    /// Width over height of the source and result tiles, 0.56…1.78.
    pub aspect: f64,
}

/// The output tile (`ResultTile`): a frame once it is saved, the signature shimmer while it renders, or an icon for a
/// job that stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResultTile {
    /// A frame of the saved result, with a check.
    Done { path: Option<String> },
    /// A pause in the warning color.
    Paused,
    /// A warning triangle in the danger color: failed or refused.
    Failed,
    /// An ✕ in the faint color.
    Canceled,
    /// The signature gradient with a sweep: queued, uploading, generating or saving.
    Rendering,
}

/// A question with its confirm button; the other button is "Not now" or "Cancel".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueConfirmation {
    pub title: String,
    pub message: String,
    pub confirm_title: String,
}

/// A batch under one header.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueBatch {
    pub id: String,
    pub title: String,
    pub progress: f64,
    pub progress_text: String,
    pub total_cost_text: Option<String>,
    pub is_active: bool,
    /// Cancel All shows: a job canceling still saves is in the batch.
    pub can_cancel_all: bool,
    pub rows: Vec<QueueRow>,
}

/// A job on its own, or a batch.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueueEntryModel {
    Job { row: Box<QueueRow> },
    Batch { batch: QueueBatch },
}

/// Everything the Queue screen and the sidebar badge show, at one moment.
#[allow(clippy::struct_excessive_bools, reason = "independent flags the screen shows")]
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub active_count: i32,
    /// "1 of 2 slots busy".
    pub slots_summary: String,
    /// A job is queued or processing: the slots pill pulses.
    pub is_busy: bool,
    pub can_pause_all: bool,
    pub can_resume_all: bool,
    /// Pause all or Resume all is running.
    pub is_changing_all: bool,
    pub active: Vec<QueueEntryModel>,
    /// Didn't finish.
    pub problems: Vec<QueueEntryModel>,
    pub finished: Vec<QueueEntryModel>,
}

/// What Duplicate did.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DuplicateOutcome {
    /// "Final queued · ~$0.68", for the toast.
    Queued { notice: String },
    /// Over today's cap: ask, then duplicate again with the cap approved.
    Confirm { confirmation: QueueConfirmation },
    /// The reason is the connection alert.
    Failed,
}

/// The progress of a job's current step.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: String,
    pub fraction: f64,
}

/// The Queue: live jobs, the problems section, and the commands.
pub struct QueueService {
    engine: JobEngine,
    store: Arc<Store>,
    preferences: Arc<PreferencesStore>,
    connection: Arc<ConnectionStore>,
    provider: ProviderHandle,
    offset: UtcOffset,
    image_spend: Arc<crate::images::ImageSpendLog>,
    live: Mutex<LiveJobs>,
    is_changing_all: AtomicBool,
    changes: watch::Sender<u64>,
    progress: broadcast::Sender<JobProgress>,
}

impl std::fmt::Debug for QueueService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("QueueService").finish_non_exhaustive()
    }
}

impl QueueService {
    pub fn new(
        engine: JobEngine,
        store: Arc<Store>,
        preferences: Arc<PreferencesStore>,
        connection: Arc<ConnectionStore>,
        provider: ProviderHandle,
        offset: UtcOffset,
        image_spend: Arc<crate::images::ImageSpendLog>,
    ) -> Self {
        Self {
            engine,
            store,
            preferences,
            connection,
            provider,
            offset,
            image_spend,
            live: Mutex::new(LiveJobs::default()),
            is_changing_all: AtomicBool::new(false),
            changes: watch::channel(0).0,
            progress: broadcast::channel(256).0,
        }
    }

    /// A revision that goes up whenever a job changes state or anything else the Queue lists changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// The progress of each job's current step, as the engine reports it.
    pub fn subscribe_progress(&self) -> broadcast::Receiver<JobProgress> {
        self.progress.subscribe()
    }

    /// Jobs in the queue now, paused ones included.
    pub fn active_count(&self) -> usize {
        self.locked().active_count()
    }

    /// Loads the active jobs, the newest finished ones and the generation history from the store. Never replaces a
    /// job an event already brought.
    pub async fn seed(&self) {
        let active = self.store.active_jobs().await;
        let finished = self.store.finished_jobs().await;
        let history = self.store.done_jobs(LiveJobs::GENERATION_HISTORY_LIMIT).await;
        {
            let mut live = self.locked();
            match active {
                Ok(jobs) => live.seed(&jobs),
                Err(error) => tracing::error!(%error, "can't load the active jobs"),
            }
            match finished {
                Ok(jobs) => {
                    let newest: Vec<Job> = jobs.into_iter().take(LiveJobs::STORED_FINISHED_LIMIT).collect();
                    live.seed(&newest);
                }
                Err(error) => tracing::error!(%error, "can't load the finished jobs"),
            }
            match history {
                Ok(jobs) => live.add_history(&jobs),
                Err(error) => tracing::error!(%error, "can't load the generation history"),
            }
        }
        self.notify();
    }

    /// Applies an engine event. Returns the job when it just finished, for the caller's follow-up (balance).
    pub fn apply(&self, event: EngineEvent, now: OffsetDateTime) -> Option<Job> {
        match event {
            EngineEvent::JobChanged(job) => {
                let job = *job;
                let finished = job.state.is_terminal().then(|| job.clone());
                let is_new_state = self.locked().apply(job, now, self.offset);
                if is_new_state {
                    self.notify();
                    return finished;
                }
                None
            }
            EngineEvent::Progress {
                job_id,
                state,
                fraction,
            } => {
                self.locked().apply_progress(job_id, &state, fraction);
                let _ = self.progress.send(JobProgress {
                    job_id: job_id.to_string(),
                    fraction: fraction.clamp(0.0, 1.0),
                });
                None
            }
        }
    }

    /// The Queue at `now`. The "Didn't finish" section reads the store, so it keeps jobs however many finish after
    /// them.
    pub async fn snapshot(&self, now: OffsetDateTime) -> QueueSnapshot {
        let problems = self.problems();
        let stored = self
            .store
            .problem_jobs(problems.cleared_at, QueueProblems::STORED_LIMIT)
            .await
            .unwrap_or_else(|error| {
                tracing::error!(%error, "can't load the jobs that didn't finish");
                Vec::new()
            });
        let kind = self.connection.state().kind();
        let limit = self.preferences.get().max_concurrency(kind);
        let live = self.locked();
        let model = |entries: Vec<QueueEntry>| -> Vec<QueueEntryModel> {
            entries
                .into_iter()
                .map(|entry| Self::entry_model(entry, &live, now))
                .collect()
        };
        QueueSnapshot {
            active_count: count(live.active_count()),
            slots_summary: live.slots_summary(limit),
            is_busy: live.busy_slots() > 0,
            can_pause_all: live.can_pause_all(),
            can_resume_all: live.can_resume_all(),
            is_changing_all: self.is_changing_all.load(Ordering::SeqCst),
            active: model(live.active_entries()),
            problems: model(live.problem_entries(&stored, &problems)),
            finished: model(live.recent_finished_entries(FINISHED_LIMIT)),
        }
    }

    fn entry_model(entry: QueueEntry, live: &LiveJobs, now: OffsetDateTime) -> QueueEntryModel {
        match entry {
            QueueEntry::Job(job) => QueueEntryModel::Job {
                row: Box::new(Self::row(&job, live, now)),
            },
            QueueEntry::Batch(batch) => QueueEntryModel::Batch {
                batch: QueueBatch {
                    id: batch.id.to_string(),
                    title: batch.title(),
                    progress: batch.progress(),
                    progress_text: batch.progress_text(),
                    total_cost_text: batch.total_cost_text(),
                    is_active: batch.is_active(),
                    can_cancel_all: !BatchCancel::new(&batch).is_empty(),
                    rows: batch.jobs.iter().map(|job| Self::row(job, live, now)).collect(),
                },
            },
        }
    }

    fn row(job: &Job, live: &LiveJobs, now: OffsetDateTime) -> QueueRow {
        let typical = live.generation_times.estimate(&job.spec);
        let model = JobRowModel::new(job, now, typical.as_ref());
        let price_confirmation = model
            .actions
            .contains(&RowAction::ConfirmPrice)
            .then(|| price_confirmation(job, &model));
        QueueRow {
            commands: QueueJobCommands::new(&model),
            support_ids: support_ids(job),
            preview: preview(job),
            price_confirmation,
            is_fresh: job.state == jutsu_core::JobState::Done
                && job
                    .finished_at
                    .is_some_and(|finished| (now - finished).as_seconds_f64() < FRESH_SECONDS),
            prompt_hint: prompt_hint(job),
            model,
        }
    }

    /// Runs `command` on job `id`; a refusal becomes the connection alert.
    pub async fn perform(&self, id: Uuid, command: JobCommand) {
        let result = match command {
            JobCommand::Cancel => self.engine.cancel(id).await,
            JobCommand::Retry => self.engine.retry(id).await,
            JobCommand::CheckAgain => self.engine.check_again(id).await,
            JobCommand::Resume => self.engine.resume(id).await,
            JobCommand::RunNext => self.engine.run_next(id).await,
        };
        if let Err(error) = result {
            let kind = self.locked().job(id).map(|job| job.provider_kind);
            self.alert(&error, kind);
        }
    }

    /// Cancel All on batch `id`: asks first once a job is with Higgsfield (call again `approved`), then cancels each
    /// job through the engine; the refusals become one alert.
    pub async fn cancel_all(&self, id: Uuid, approved: bool) -> Option<QueueConfirmation> {
        let cancel = self
            .locked()
            .active_entries()
            .into_iter()
            .find_map(|entry| match entry {
                QueueEntry::Batch(batch) if batch.id == id => Some(BatchCancel::new(&batch)),
                QueueEntry::Batch(_) | QueueEntry::Job(_) => None,
            })?;
        if !approved && let Some(confirmation) = cancel.confirmation() {
            return Some(confirmation);
        }
        let mut refusals = Vec::new();
        for job in &cancel.jobs {
            if let Err(error) = self.engine.cancel(job.id).await
                && let Some(message) = user_message(&error, Some(job.provider_kind))
            {
                refusals.push(message);
            }
        }
        if let Some(message) = BatchCancel::outcome(&refusals, cancel.jobs.len()) {
            self.connection.show_alert(message);
        }
        None
    }

    /// The person's Pause all: every job not submitted yet, agents' included.
    pub async fn pause_all(&self) {
        Box::pin(self.changing_all(self.engine.pause_all(false))).await;
    }

    /// The person's Resume all.
    pub async fn resume_all(&self) {
        Box::pin(self.changing_all(self.engine.resume_all(false))).await;
    }

    async fn changing_all(&self, work: impl Future<Output = ()>) {
        self.is_changing_all.store(true, Ordering::SeqCst);
        self.notify();
        work.await;
        self.is_changing_all.store(false, Ordering::SeqCst);
        self.notify();
    }

    /// Hides job `id` from "Didn't finish".
    pub async fn remove_problem(&self, id: Uuid) -> Result<(), crate::preferences::PreferencesError> {
        let mut problems = self.problems();
        let loaded: HashSet<Uuid> = self
            .store
            .problem_jobs(problems.cleared_at, QueueProblems::STORED_LIMIT)
            .await
            .map(|jobs| jobs.into_iter().map(|job| job.id).collect())
            .unwrap_or_default();
        problems.remove(id, &loaded);
        self.save_problems(&problems)
    }

    /// "Clear all": hides every job that didn't finish until now.
    pub fn clear_problems(&self, now: OffsetDateTime) -> Result<(), crate::preferences::PreferencesError> {
        let mut problems = self.problems();
        problems.clear_all(now);
        self.save_problems(&problems)
    }

    fn problems(&self) -> QueueProblems {
        let preferences = self.preferences.get();
        QueueProblems::new(
            preferences.queue_problems_cleared_at,
            preferences.queue_removed_problems.as_deref(),
        )
    }

    fn save_problems(&self, problems: &QueueProblems) -> Result<(), crate::preferences::PreferencesError> {
        self.preferences.update(crate::Preferences {
            queue_problems_cleared_at: Some(problems.stored_cleared_at().unwrap_or(0.0)),
            queue_removed_problems: Some(problems.stored_removed().unwrap_or_default()),
            ..crate::Preferences::default()
        })?;
        self.notify();
        Ok(())
    }

    /// Queues job `id`'s request as a new job. Over today's spend cap it asks first, unless `over_cap_approved`.
    pub async fn duplicate(
        &self,
        id: Uuid,
        over_cap_approved: bool,
        now: OffsetDateTime,
    ) -> DuplicateOutcome {
        let live = self.locked().job(id).cloned();
        let job = match live {
            Some(job) => Some(job),
            None => self.store.job(id).await.ok().flatten(),
        };
        let Some(job) = job else {
            self.connection.show_alert(UserMessage::new(
                "Can't duplicate this job",
                "This job is no longer in the queue.",
                None,
            ));
            return DuplicateOutcome::Failed;
        };
        let spec = job.spec.clone();
        let kind = self.connection.state().kind();
        if !over_cap_approved {
            match self.submit_decision(&spec, kind, now).await {
                Ok(None) => {}
                Ok(Some(confirmation)) => return DuplicateOutcome::Confirm { confirmation },
                Err(reason) => {
                    self.connection
                        .show_alert(UserMessage::new("Can't duplicate this job", reason, None));
                    return DuplicateOutcome::Failed;
                }
            }
        }
        let options = EnqueueOptions {
            over_cap_approved,
            ..EnqueueOptions::default()
        };
        match self.engine.enqueue(spec, options).await {
            Ok(queued) => DuplicateOutcome::Queued {
                notice: queued_notice(&[queued]).unwrap_or_default(),
            },
            Err(error) => {
                self.alert(&error, kind);
                DuplicateOutcome::Failed
            }
        }
    }

    /// `Ok(None)` starts at once, `Ok(Some)` asks first, `Err` is the blocker text (`SubmitDecision.budgeted`).
    async fn submit_decision(
        &self,
        spec: &JobSpec,
        kind: Option<ProviderKind>,
        now: OffsetDateTime,
    ) -> Result<Option<QueueConfirmation>, String> {
        let Some(cap) = self.preferences.get().spend_cap(kind) else {
            return Ok(None);
        };
        let committed = crate::settings::spending::committed_today(
            &self.store,
            &self.image_spend,
            cap.unit,
            now,
            self.offset,
        )
        .await?;
        let cost = match self.provider.current() {
            Some(provider) => provider.estimate(spec).await.ok(),
            None => None,
        };
        Ok(budgeted(
            JobSpec::job_count(std::slice::from_ref(spec)),
            cost.as_ref(),
            committed,
            &cap,
        ))
    }

    /// The job's result file, for Show in File Manager.
    pub fn output_path(&self, id: Uuid) -> Option<std::path::PathBuf> {
        self.locked()
            .job(id)
            .and_then(|job| job.output_file.as_ref())
            .and_then(|url| url.to_file_path().ok())
    }

    fn alert(&self, error: &EngineError, kind: Option<ProviderKind>) {
        if let Some(message) = user_message(error, kind) {
            self.connection.show_alert(message);
        }
    }

    fn notify(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    fn locked(&self) -> MutexGuard<'_, LiveJobs> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a refused command tells the person; `None` for a cancelled call.
pub fn user_message(error: &EngineError, kind: Option<ProviderKind>) -> Option<UserMessage> {
    match error {
        EngineError::Cancelled => None,
        EngineError::Port(port) => UserMessage::from_error(port, kind),
        // The engine's text names a Mac.
        EngineError::Offline => Some(UserMessage::failure(
            "This computer is offline. Try again once it is connected.",
        )),
        other => UserMessage::from_error(&PortError::Other(other.to_string()), kind),
    }
}

/// Start (`None`), or the question to ask first when `cost`, the total of `jobs` jobs, takes today's spend over
/// `cap`. An unknown cost asks too, since it could be over.
pub fn budgeted(
    jobs: i64,
    cost: Option<&Cost>,
    committed_today: Decimal,
    cap: &SpendCap,
) -> Option<QueueConfirmation> {
    let cap_text = format::amount(cap.amount, cap.unit);
    let Some(cost) = cost.filter(|cost| !cost.is_unknown_price()) else {
        let today = format::amount(committed_today, cap.unit);
        let message = if jobs == 1 {
            format!(
                "This job's price isn't known, so it may take today past your {cap_text} cap. Today is at {today} so far."
            )
        } else {
            format!(
                "These {jobs} jobs' price isn't known, so it may take today past your {cap_text} cap. Today is at {today} so far."
            )
        };
        return Some(QueueConfirmation {
            title: "Price not known yet".to_owned(),
            message,
            confirm_title: "Start anyway".to_owned(),
        });
    };
    let BudgetDecision::NeedsConfirmation { total } = Budget::allows(cost, committed_today, Some(cap)) else {
        return None;
    };
    let amount = format::amount(cost.amount(), cap.unit);
    let total = format::amount(total, cap.unit);
    let message = if jobs == 1 {
        format!("This job adds {amount}, bringing today to {total} against your {cap_text} cap.")
    } else {
        format!("These {jobs} jobs add {amount}, bringing today to {total} against your {cap_text} cap.")
    };
    Some(QueueConfirmation {
        title: "Over today's spend cap".to_owned(),
        message,
        confirm_title: "Start anyway".to_owned(),
    })
}

/// Review price's question: the price from Higgsfield, or why an agent's job waits for the person.
fn price_confirmation(job: &Job, model: &JobRowModel) -> QueueConfirmation {
    let price = &model.cost;
    let message = if job.state == jutsu_core::JobState::Paused(PauseReason::AgentLimit) {
        format!(
            "An agent queued this job: {}",
            model
                .note
                .as_ref()
                .map(|note| note.text.as_str())
                .unwrap_or_default()
        )
    } else {
        "This is the price from Higgsfield for the uploaded files. It takes today's spending over your cap."
            .to_owned()
    };
    QueueConfirmation {
        title: format!("Start this job for {price}?"),
        message,
        confirm_title: format!("Start for {price}"),
    }
}

/// "Final queued · ~$0.68" (`QueuedNotice`); `None` when nothing was queued.
pub fn queued_notice(jobs: &[Job]) -> Option<String> {
    let first = jobs.first()?;
    let title = if jobs.len() > 1 {
        format!("{} jobs queued", jobs.len())
    } else if first.spec.is_draft {
        "Draft queued".to_owned()
    } else {
        "Final queued".to_owned()
    };
    let unknown: Vec<&Job> = jobs.iter().filter(|job| job.price_is_unknown()).collect();
    let price = if unknown.is_empty() {
        let prices: Vec<Cost> = jobs.iter().map(|job| job.price().clone()).collect();
        format::total(&prices).map(|total| format::format_cost(&total, true))
    } else if unknown.iter().all(|job| {
        job.spec
            .operation
            .as_ref()
            .is_some_and(|operation| operation.action.is_priced_after_run())
    }) {
        Some(format::PRICE_AFTER_RUN.to_owned())
    } else {
        Some(format::PRICE_UNKNOWN.to_owned())
    };
    Some(
        [Some(title), price]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

/// The prompt as written, for the tooltip of the row's one-line prompt; `None` for a job without one.
pub fn prompt_hint(job: &Job) -> Option<String> {
    (!job.spec.prompt.trim().is_empty()).then(|| job.spec.prompt.clone())
}

/// The IDs Higgsfield support asks for when a job went wrong; `None` for any other job. A missing field gets
/// "none", so the line always has every field.
pub fn support_ids(job: &Job) -> Option<String> {
    if !matches!(
        job.state,
        jutsu_core::JobState::Failed { .. } | jutsu_core::JobState::Rejected
    ) {
        return None;
    }
    let remote = job.remote.as_ref()?;
    Some(format!(
        "request_id={} correlation_id={} app={} build=none os={}",
        remote.request_id,
        remote.correlation_id.as_deref().unwrap_or("none"),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
    ))
}

fn preview(job: &Job) -> JobPreview {
    let path = |url: &url::Url| {
        url.to_file_path()
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    };
    let references = &job.spec.references;
    // The source's shape, or the frame asked for without one, kept within reason.
    let aspect = match &job.spec.source {
        Some(source) if source.pixel_size.height > 0 => {
            ratio(source.pixel_size.width, source.pixel_size.height)
        }
        Some(_) => 16.0 / 9.0,
        None => job
            .spec
            .aspect_ratio
            .and_then(jutsu_core::AspectRatio::width_over_height)
            .unwrap_or(16.0 / 9.0),
    }
    .clamp(0.56, 1.78);
    let result = match &job.state {
        jutsu_core::JobState::Done => ResultTile::Done {
            path: job.output_file.as_ref().and_then(path),
        },
        jutsu_core::JobState::Paused(_) => ResultTile::Paused,
        jutsu_core::JobState::Failed { .. } | jutsu_core::JobState::Rejected => ResultTile::Failed,
        jutsu_core::JobState::Canceled => ResultTile::Canceled,
        _ => ResultTile::Rendering,
    };
    JobPreview {
        has_source: job.spec.source.is_some(),
        source: job.spec.source.as_ref().and_then(|source| path(&source.file_url)),
        more_references: count(references.len().saturating_sub(3)),
        references: references
            .iter()
            .take(3)
            .map(|image| path(&image.file_url))
            .collect(),
        result,
        aspect,
    }
}

#[allow(clippy::cast_precision_loss, reason = "frame sizes are far below 2^52")]
fn ratio(width: i64, height: i64) -> f64 {
    width as f64 / height as f64
}

fn count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The live jobs as agents read them (`environment.liveJobs` in `AgentService+Jobs.swift`).
impl QueueService {
    /// The job with `id` while the Queue holds it.
    pub fn live_job(&self, id: Uuid) -> Option<Job> {
        self.locked().job(id).cloned()
    }

    /// Every job the Queue holds that an agent queued.
    pub fn agent_jobs(&self) -> Vec<Job> {
        self.locked()
            .all()
            .into_iter()
            .filter(|job| job.agent.is_some())
            .cloned()
            .collect()
    }
}

/// The live jobs as the system tray, the notifications and the taskbar read them.
impl QueueService {
    /// Runs `read` on every job this session knows, under the lock; keep it short.
    pub fn with_live<T>(&self, read: impl FnOnce(&LiveJobs) -> T) -> T {
        read(&self.locked())
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
