//! The latest known version of every job this session has seen (`LiveJobs.swift`), the Queue's entries
//! (`JobBatch.swift`) and its "Didn't finish" section (`QueueProblems.swift`).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use jutsu_core::support::date::start_of_day;
use jutsu_core::{GenerationTimes, Job, JobSpec, JobState, Mode, PauseReason};
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::format;
use super::row::{Stage, is_not_charged, reports_progress};
use crate::connection::mode_title;

/// Every job this session knows, kept current from the engine's events. Shared by the Queue screen and the
/// sidebar badge.
#[derive(Debug, Clone, Default)]
pub struct LiveJobs {
    jobs: HashMap<Uuid, Job>,
    /// How long past jobs took to generate, from the loaded history and every job finished since.
    pub generation_times: GenerationTimes,
}

impl LiveJobs {
    /// How many finished jobs the app loads on launch, and keeps of those finished before today.
    pub const STORED_FINISHED_LIMIT: usize = 50;
    /// How many done jobs the app loads on launch for `generation_times`.
    pub const GENERATION_HISTORY_LIMIT: u32 = 500;

    pub fn new(jobs: &[Job]) -> Self {
        let mut live = Self::default();
        live.seed(jobs);
        live
    }

    /// Every job this session knows, in no order.
    pub fn all(&self) -> Vec<&Job> {
        self.jobs.values().collect()
    }

    pub fn job(&self, id: Uuid) -> Option<&Job> {
        self.jobs.get(&id)
    }

    /// Jobs still in the queue, paused ones included, oldest first.
    pub fn active(&self) -> Vec<&Job> {
        let mut active: Vec<&Job> = self.jobs.values().filter(|job| job.state.is_active()).collect();
        active.sort_by(|lhs, rhs| oldest_first(lhs, rhs));
        active
    }

    pub fn active_count(&self) -> usize {
        self.jobs.values().filter(|job| job.state.is_active()).count()
    }

    /// Jobs whose submit is out and not answered yet; quitting now could cut it off.
    pub fn submitting_count(&self) -> usize {
        self.jobs
            .values()
            .filter(|job| job.state == JobState::WaitingForSlot && job.submit_attempt.is_some())
            .count()
    }

    /// Jobs the API has accepted and not finished; each holds one of the account's generation slots.
    pub fn busy_slots(&self) -> usize {
        self.jobs
            .values()
            .filter(|job| matches!(job.state, JobState::Queued | JobState::Processing))
            .count()
    }

    /// Pause all has a job to stop before it is submitted.
    pub fn can_pause_all(&self) -> bool {
        self.jobs.values().any(|job| job.state.is_pausable_by_user())
    }

    /// Resume all has a job the user paused.
    pub fn can_resume_all(&self) -> bool {
        self.jobs
            .values()
            .any(|job| job.state == JobState::Paused(PauseReason::ByUser))
    }

    /// "1 of 2 slots busy".
    pub fn slots_summary(&self, limit: i64) -> String {
        format!("{} of {limit} slots busy", self.busy_slots())
    }

    /// Done, failed, rejected and canceled jobs, most recently finished first.
    pub fn recent_finished(&self, limit: usize) -> Vec<&Job> {
        let mut finished: Vec<&Job> = self.jobs.values().filter(|job| job.state.is_terminal()).collect();
        finished.sort_by(|lhs, rhs| newest_finished_first(lhs, rhs));
        finished.truncate(limit);
        finished
    }

    /// The Active section: active jobs, oldest first, each batch under one header at its first job. A batch with
    /// any active job is shown here whole, finished jobs included, so its progress counts them.
    pub fn active_entries(&self) -> Vec<QueueEntry> {
        let batches = self.active_batch_ids();
        let mut shown: Vec<&Job> = self
            .jobs
            .values()
            .filter(|job| job.state.is_active() || job.batch_id.is_some_and(|id| batches.contains(&id)))
            .collect();
        shown.sort_by(|lhs, rhs| oldest_first(lhs, rhs));
        QueueEntry::group(shown.into_iter().cloned().collect())
    }

    /// The Finished section: the done jobs without the batches still shown as active, at most `limit` entries, a
    /// batch counting as one.
    pub fn recent_finished_entries(&self, limit: usize) -> Vec<QueueEntry> {
        let batches = self.active_batch_ids();
        let finished: Vec<Job> = self
            .recent_finished(usize::MAX)
            .into_iter()
            .filter(|job| {
                job.state == JobState::Done && !job.batch_id.is_some_and(|id| batches.contains(&id))
            })
            .cloned()
            .collect();
        let mut entries = QueueEntry::group(finished);
        entries.truncate(limit);
        entries
    }

    /// The "Didn't finish" section: the problems among `stored` and the live jobs, each in its latest known version,
    /// newest finished first, a batch under one header. Leaves out the batches Active shows whole.
    pub fn problem_entries(&self, stored: &[Job], problems: &QueueProblems) -> Vec<QueueEntry> {
        let mut latest: HashMap<Uuid, &Job> = HashMap::new();
        for job in stored {
            latest.insert(job.id, self.job(job.id).unwrap_or(job));
        }
        for job in self.recent_finished(usize::MAX) {
            if QueueProblems::is_problem(&job.state) {
                latest.insert(job.id, job);
            }
        }
        let batches = self.active_batch_ids();
        let mut shown: Vec<&Job> = latest
            .into_values()
            .filter(|job| problems.shows(job) && !job.batch_id.is_some_and(|id| batches.contains(&id)))
            .collect();
        shown.sort_by(|lhs, rhs| newest_finished_first(lhs, rhs));
        QueueEntry::group(shown.into_iter().cloned().collect())
    }

    /// Batches with a job still in the queue; Active shows them whole.
    fn active_batch_ids(&self) -> HashSet<Uuid> {
        self.jobs
            .values()
            .filter(|job| job.state.is_active())
            .filter_map(|job| job.batch_id)
            .collect()
    }

    /// Replaces the job with the same id: the engine always sends the whole job. Returns whether its state changed.
    pub fn apply(&mut self, job: Job, now: OffsetDateTime, offset: UtcOffset) -> bool {
        let is_new_state = self.jobs.get(&job.id).map(|known| &known.state) != Some(&job.state);
        if self.generation_times.needs_recording(&job) {
            self.generation_times.record(std::slice::from_ref(&job));
        }
        let is_terminal = job.state.is_terminal();
        self.jobs.insert(job.id, job);
        if is_new_state && is_terminal {
            let old = Self::old_finished(self.jobs.values(), Self::STORED_FINISHED_LIMIT, now, offset);
            self.jobs.retain(|id, _| !old.contains(id));
        }
        is_new_state
    }

    /// The progress of a job's current step, which the engine doesn't save; ignored for another state.
    pub fn apply_progress(&mut self, id: Uuid, state: &JobState, fraction: f64) {
        if let Some(job) = self.jobs.get_mut(&id)
            && job.state == *state
        {
            job.progress = fraction;
        }
    }

    /// Adds jobs loaded from storage. Never replaces one already here: an engine event may have arrived first and is
    /// newer than the stored copy.
    pub fn seed(&mut self, stored: &[Job]) {
        for job in stored {
            self.jobs.entry(job.id).or_insert_with(|| job.clone());
        }
        self.add_history(stored);
    }

    /// Adds the timings of finished jobs loaded from storage to `generation_times`.
    pub fn add_history(&mut self, history: &[Job]) {
        let new: Vec<Job> = history
            .iter()
            .filter(|job| self.generation_times.needs_recording(job))
            .cloned()
            .collect();
        if !new.is_empty() {
            self.generation_times.record(&new);
        }
    }

    /// Finished jobs past the newest `keep` that finished before `now`'s day, whose batch has nothing else left:
    /// today's spend counts the rest, and the Queue shows a batch whole.
    pub fn old_finished<'a>(
        jobs: impl Iterator<Item = &'a Job> + Clone,
        keep: usize,
        now: OffsetDateTime,
        offset: UtcOffset,
    ) -> HashSet<Uuid> {
        let today = start_of_day(now, offset);
        let mut finished: Vec<&Job> = jobs.clone().filter(|job| job.state.is_terminal()).collect();
        finished.sort_by(|lhs, rhs| newest_finished_first(lhs, rhs));
        let old: Vec<&Job> = finished
            .into_iter()
            .skip(keep)
            .filter(|job| job.finished_at.unwrap_or(job.created_at) < today)
            .collect();
        let dropped: HashSet<Uuid> = old.iter().map(|job| job.id).collect();
        let kept_batches: HashSet<Uuid> = jobs
            .filter(|job| !dropped.contains(&job.id))
            .filter_map(|job| job.batch_id)
            .collect();
        old.into_iter()
            .filter(|job| !job.batch_id.is_some_and(|id| kept_batches.contains(&id)))
            .map(|job| job.id)
            .collect()
    }
}

/// Oldest first; the ID settles a tie (`LiveJobs.active`).
pub fn oldest_first(lhs: &Job, rhs: &Job) -> Ordering {
    (lhs.created_at, uuid_text(lhs)).cmp(&(rhs.created_at, uuid_text(rhs)))
}

/// Most recently finished first; a job not finished counts from its creation.
pub fn newest_finished_first(lhs: &Job, rhs: &Job) -> Ordering {
    let left = lhs.finished_at.unwrap_or(lhs.created_at);
    let right = rhs.finished_at.unwrap_or(rhs.created_at);
    right.cmp(&left).then_with(|| uuid_text(rhs).cmp(&uuid_text(lhs)))
}

/// Swift compares `uuidString`, upper case.
fn uuid_text(job: &Job) -> String {
    job.id.as_hyphenated().to_string().to_uppercase()
}

/// One row of the Queue: a job on its own, or a batch with its jobs under one header.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueEntry {
    Job(Box<Job>),
    Batch(JobBatch),
}

impl QueueEntry {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Job(job) => job.id,
            Self::Batch(batch) => batch.id,
        }
    }

    /// The job, or the batch's jobs.
    pub fn jobs(&self) -> Vec<&Job> {
        match self {
            Self::Job(job) => vec![job],
            Self::Batch(batch) => batch.jobs.iter().collect(),
        }
    }

    /// `jobs` in order, a batch at the place of its first job. A batch with a single member in the list stays a
    /// plain job: a group of one compares nothing.
    pub fn group(jobs: Vec<Job>) -> Vec<Self> {
        let mut members: HashMap<Uuid, Vec<Job>> = HashMap::new();
        for job in &jobs {
            if let Some(id) = job.batch_id {
                members.entry(id).or_default().push(job.clone());
            }
        }
        let mut placed = HashSet::new();
        let mut entries = Vec::new();
        for job in jobs {
            match job.batch_id {
                Some(id) if members.get(&id).is_some_and(|batch| batch.len() > 1) => {
                    if placed.insert(id) {
                        let batch = members.get(&id).cloned().unwrap_or_default();
                        entries.push(Self::Batch(JobBatch::new(id, batch)));
                    }
                }
                _ => entries.push(Self::Job(Box::new(job))),
            }
        }
        entries
    }
}

/// The jobs of one batch under one Queue header, with their combined progress.
#[derive(Debug, Clone, PartialEq)]
pub struct JobBatch {
    pub id: Uuid,
    /// In the order they were queued.
    pub jobs: Vec<Job>,
}

impl JobBatch {
    pub fn new(id: Uuid, mut jobs: Vec<Job>) -> Self {
        jobs.sort_by(oldest_first);
        Self { id, jobs }
    }

    /// "dance · Restyle batch · 4 styles".
    pub fn title(&self) -> String {
        let Some(first) = self.jobs.first() else {
            return "Batch".to_owned();
        };
        let specs: Vec<&JobSpec> = self.jobs.iter().map(|job| &job.spec).collect();
        format!(
            "{} · {} batch · {}",
            first.spec.name(),
            mode_title(first.spec.mode),
            variant_text(self.jobs.len(), &specs)
        )
    }

    pub fn is_active(&self) -> bool {
        self.jobs.iter().any(|job| job.state.is_active())
    }

    /// 0…1 over all jobs: a finished job counts whole, a running one by its step and that step's progress.
    pub fn progress(&self) -> f64 {
        if self.jobs.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.jobs.iter().map(job_progress).sum();
        sum / count_f64(self.jobs.len())
    }

    /// "2 of 4 done", "2 of 4 done · 1 stopped".
    pub fn progress_text(&self) -> String {
        let done = self.jobs.iter().filter(|job| job.state == JobState::Done).count();
        let stopped = self
            .jobs
            .iter()
            .filter(|job| job.state.is_terminal() && job.state != JobState::Done)
            .count();
        let base = format!("{done} of {} done", self.jobs.len());
        if stopped == 0 {
            base
        } else {
            format!("{base} · {stopped} stopped")
        }
    }

    /// What the batch costs so far: the best known prices of the jobs that are or will be charged. `None` when
    /// none is, or when they are paid in different units.
    pub fn total_cost_text(&self) -> Option<String> {
        let costs: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| !is_not_charged(job))
            .map(|job| job.price().clone())
            .collect();
        format::total(&costs).map(|total| format::format_cost(&total, true))
    }
}

fn job_progress(job: &Job) -> f64 {
    if job.state.is_terminal() {
        return 1.0;
    }
    let Some(stage) = Stage::of(&job.state) else {
        return 0.0;
    };
    let within = if reports_progress(&job.state) {
        job.progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (stage.index() + within) / Stage::COUNT
}

#[allow(clippy::cast_precision_loss, reason = "a batch has far fewer than 2^52 jobs")]
fn count_f64(count: usize) -> f64 {
    count as f64
}

/// "4 styles" or "3 characters", after the mode the batch varies; "3 variants" for the jobs of one submit of
/// variants.
fn variant_text(count: usize, specs: &[&JobSpec]) -> String {
    if are_variants(specs) {
        return format!("{count} variants");
    }
    if specs.first().map_or(Mode::Restyle, |spec| spec.mode) == Mode::Restyle {
        format!("{count} styles")
    } else {
        format!("{count} characters")
    }
}

/// The jobs of one submit of variants share every input.
fn are_variants(specs: &[&JobSpec]) -> bool {
    let Some(first) = specs.first() else {
        return false;
    };
    specs.len() > 1
        && first.mode != Mode::Restyle
        && specs.iter().all(|spec| {
            spec.references == first.references
                && spec.preset_id == first.preset_id
                && spec.source == first.source
                && spec.prompt == first.prompt
        })
}

/// The "Didn't finish" section's state: what "Clear all" and Remove hid. Removing hides a job from the Queue only;
/// its record, its cost in today's spend and its IDs stay in the store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueProblems {
    /// Jobs that finished at or before this were cleared with "Clear all"; `None` when never cleared.
    pub cleared_at: Option<OffsetDateTime>,
    /// Jobs removed one by one since `cleared_at`.
    pub removed: HashSet<Uuid>,
}

impl QueueProblems {
    /// How many such jobs the Queue loads from the store, newest first, removed ones included.
    pub const STORED_LIMIT: u32 = 100;

    /// From the stored preferences: seconds since 1970 (0 or `None` for never), and ids separated by commas.
    pub fn new(cleared_at: Option<f64>, removed: Option<&str>) -> Self {
        Self {
            cleared_at: cleared_at
                .filter(|seconds| *seconds > 0.0)
                .map(jutsu_core::support::date::from_unix_seconds),
            removed: removed
                .unwrap_or_default()
                .split(',')
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect(),
        }
    }

    /// `cleared_at` as stored, seconds since 1970, `None` for never.
    pub fn stored_cleared_at(&self) -> Option<f64> {
        self.cleared_at.map(jutsu_core::support::date::unix_seconds)
    }

    /// `removed` as stored: upper-case ids separated by commas, sorted so an unchanged set writes the same value.
    pub fn stored_removed(&self) -> Option<String> {
        if self.removed.is_empty() {
            return None;
        }
        let mut ids: Vec<String> = self
            .removed
            .iter()
            .map(|id| id.as_hyphenated().to_string().to_uppercase())
            .collect();
        ids.sort();
        Some(ids.join(","))
    }

    /// The jobs the section is about: finished, but not done.
    pub fn is_problem(state: &JobState) -> bool {
        state.is_terminal() && *state != JobState::Done
    }

    /// Whether the section shows `job`: a problem the user hasn't cleared or removed.
    pub fn shows(&self, job: &Job) -> bool {
        if !Self::is_problem(&job.state) || self.removed.contains(&job.id) {
            return false;
        }
        self.cleared_at
            .is_none_or(|cleared_at| job.finished_at.unwrap_or(job.created_at) > cleared_at)
    }

    /// Hides `id`. Forgets removed ids no longer among `loaded`, the jobs the store returned.
    pub fn remove(&mut self, id: Uuid, loaded: &HashSet<Uuid>) {
        self.removed.retain(|removed| loaded.contains(removed));
        self.removed.insert(id);
    }

    /// Hides every job finished until `now`. Jobs that fail later, a retried one included, show again.
    pub fn clear_all(&mut self, now: OffsetDateTime) {
        self.cleared_at = Some(now);
        self.removed.clear();
    }
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
