//! `JobRecord`: the execution and spending ledger, and the job queries.

use std::collections::HashSet;

use async_trait::async_trait;
use jutsu_core::{Budget, CostUnit, Job, JobRepository, JobState, MaterialInput, MaterialRemote, PortError};
use rust_decimal::Decimal;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};
use time::{OffsetDateTime, Time, UtcOffset};
use uuid::Uuid;

use crate::catalog::{inputs_of, register_video};
use crate::error::StoreError;
use crate::store::{Store, StoreChange, date, nanos, text, uuid};

const TABLE: &str = "jobs";

/// `JobState`'s case name without its payload, as the `state_kind` column holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobStateKind {
    /// "preparing".
    Preparing,
    /// "uploading".
    Uploading,
    /// "waitingForSlot".
    WaitingForSlot,
    /// "queued".
    Queued,
    /// "processing".
    Processing,
    /// "downloading".
    Downloading,
    /// "done".
    Done,
    /// "failed".
    Failed,
    /// "rejected".
    Rejected,
    /// "canceled".
    Canceled,
    /// "paused".
    Paused,
}

impl JobStateKind {
    /// The kind of `state`.
    pub fn of(state: &JobState) -> Self {
        match state {
            JobState::Preparing => Self::Preparing,
            JobState::Uploading => Self::Uploading,
            JobState::WaitingForSlot => Self::WaitingForSlot,
            JobState::Queued => Self::Queued,
            JobState::Processing => Self::Processing,
            JobState::Downloading => Self::Downloading,
            JobState::Done => Self::Done,
            JobState::Failed { .. } => Self::Failed,
            JobState::Rejected => Self::Rejected,
            JobState::Canceled => Self::Canceled,
            JobState::Paused(_) => Self::Paused,
        }
    }

    /// As stored.
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Uploading => "uploading",
            Self::WaitingForSlot => "waitingForSlot",
            Self::Queued => "queued",
            Self::Processing => "processing",
            Self::Downloading => "downloading",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::Canceled => "canceled",
            Self::Paused => "paused",
        }
    }
}

/// A persisted `Job`. The whole job is stored as JSON in `job_data`, so `Job` stays the single source of truth;
/// the other columns are copies kept only so queries can filter and sort on them.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRecord {
    /// The job's ID.
    pub id: Uuid,
    /// When it was queued.
    pub created_at: OffsetDateTime,
    /// When it ended.
    pub finished_at: Option<OffsetDateTime>,
    /// `JobStateKind::raw_value`, e.g. "done" or "failed".
    pub state_kind: String,
    /// The state is terminal.
    pub is_terminal: bool,
    /// A draft.
    pub is_draft: bool,
    /// The mode's raw value.
    pub mode: String,
    /// The resolution's raw value.
    pub resolution: String,
    /// The connection kind's raw value.
    pub provider_kind: String,
    /// The prompt.
    pub prompt: String,
    /// The result's path.
    pub output_path: Option<String>,
    /// The job as JSON.
    pub job_data: String,
    /// Input versions pinned when first enqueued, independently of later execution updates, as JSON.
    pub input_references: String,
    /// `MaterialRemote::key` of the submitted job.
    pub remote_key: Option<String>,
}

impl JobRecord {
    /// The record of `job`, with `inputs` pinned.
    pub fn new(job: &Job, inputs: &[MaterialInput]) -> Result<Self, StoreError> {
        let mut record = Self {
            id: job.id,
            created_at: job.created_at,
            finished_at: None,
            state_kind: String::new(),
            is_terminal: false,
            is_draft: false,
            mode: String::new(),
            resolution: String::new(),
            provider_kind: String::new(),
            prompt: String::new(),
            output_path: None,
            job_data: String::new(),
            input_references: serde_json::to_string(inputs).map_err(StoreError::encode("job inputs"))?,
            remote_key: None,
        };
        record.update(job)?;
        Ok(record)
    }

    /// The stored job.
    pub fn job(&self) -> Result<Job, StoreError> {
        serde_json::from_str(&self.job_data).map_err(|error| StoreError::damaged(TABLE, self.id, error))
    }

    /// The pinned input versions.
    pub fn inputs(&self) -> Result<Vec<MaterialInput>, StoreError> {
        serde_json::from_str(&self.input_references)
            .map_err(|error| StoreError::damaged(TABLE, self.id, error))
    }

    /// Replaces the stored job and its query columns. The ID never changes, and neither do the pinned inputs.
    pub fn update(&mut self, job: &Job) -> Result<(), StoreError> {
        if job.id != self.id {
            return Err(StoreError::JobIdMismatch);
        }
        self.job_data = serde_json::to_string(job).map_err(StoreError::encode("job"))?;
        self.created_at = job.created_at;
        self.finished_at = job.finished_at;
        JobStateKind::of(&job.state)
            .raw_value()
            .clone_into(&mut self.state_kind);
        self.is_terminal = job.state.is_terminal();
        self.is_draft = job.spec.is_draft;
        job.spec.mode.raw_value().clone_into(&mut self.mode);
        job.spec.resolution.raw_value().clone_into(&mut self.resolution);
        job.provider_kind.raw_value().clone_into(&mut self.provider_kind);
        self.prompt.clone_from(&job.spec.prompt);
        self.output_path = job.output_file.as_ref().map(|url| {
            url.to_file_path().map_or_else(
                |()| url.path().to_owned(),
                |path| path.to_string_lossy().into_owned(),
            )
        });
        self.remote_key = job.remote.as_ref().map(|remote| {
            MaterialRemote {
                provider: job.provider_kind,
                owner: job.owner.clone(),
                id: remote.request_id.clone(),
                output: 0,
            }
            .key()
        });
        Ok(())
    }

    /// The record in a row of `jobs`.
    pub fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let id = text(row, "id")?;
        let finished_at: Option<i64> = row.try_get("finished_at")?;
        Ok(Self {
            id: uuid(TABLE, &id)?,
            created_at: date(TABLE, &id, row.try_get("created_at")?)?,
            finished_at: finished_at.map(|value| date(TABLE, &id, value)).transpose()?,
            state_kind: text(row, "state_kind")?,
            is_terminal: row.try_get("is_terminal")?,
            is_draft: row.try_get("is_draft")?,
            mode: text(row, "mode")?,
            resolution: text(row, "resolution")?,
            provider_kind: text(row, "provider_kind")?,
            prompt: text(row, "prompt")?,
            output_path: row.try_get("output_path")?,
            job_data: text(row, "job_data")?,
            input_references: text(row, "input_references")?,
            remote_key: row.try_get("remote_key")?,
        })
    }

    async fn insert(&self, connection: &mut SqliteConnection) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO jobs (id, created_at, finished_at, state_kind, is_terminal, is_draft, mode, resolution, \
             provider_kind, prompt, output_path, job_data, input_references, remote_key) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(self.id.to_string())
        .bind(nanos(self.created_at))
        .bind(self.finished_at.map(nanos))
        .bind(&self.state_kind)
        .bind(self.is_terminal)
        .bind(self.is_draft)
        .bind(&self.mode)
        .bind(&self.resolution)
        .bind(&self.provider_kind)
        .bind(&self.prompt)
        .bind(&self.output_path)
        .bind(&self.job_data)
        .bind(&self.input_references)
        .bind(&self.remote_key)
        .execute(connection)
        .await?;
        Ok(())
    }

    async fn write(&self, connection: &mut SqliteConnection) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE jobs SET created_at = ?, finished_at = ?, state_kind = ?, is_terminal = ?, is_draft = ?, \
             mode = ?, resolution = ?, provider_kind = ?, prompt = ?, output_path = ?, job_data = ?, \
             remote_key = ? WHERE id = ?",
        )
        .bind(nanos(self.created_at))
        .bind(self.finished_at.map(nanos))
        .bind(&self.state_kind)
        .bind(self.is_terminal)
        .bind(self.is_draft)
        .bind(&self.mode)
        .bind(&self.resolution)
        .bind(&self.provider_kind)
        .bind(&self.prompt)
        .bind(&self.output_path)
        .bind(&self.job_data)
        .bind(&self.remote_key)
        .bind(self.id.to_string())
        .execute(connection)
        .await?;
        Ok(())
    }
}

/// The ID of the job submitted as `remote_key`, if one is stored.
pub(crate) async fn job_id_by_remote_key(
    connection: &mut SqliteConnection,
    remote_key: &str,
) -> Result<Option<Uuid>, StoreError> {
    let id: Option<String> = sqlx::query_scalar("SELECT id FROM jobs WHERE remote_key = ? LIMIT 1")
        .bind(remote_key)
        .fetch_optional(connection)
        .await?;
    id.map(|id| uuid(TABLE, &id)).transpose()
}

/// The jobs of `records` that decode; the rest are logged and skipped, never deleted.
fn decoded_jobs(records: &[SqliteRow]) -> Result<Vec<Job>, StoreError> {
    let mut jobs = Vec::with_capacity(records.len());
    for row in records {
        let record = JobRecord::from_row(row)?;
        match record.job() {
            Ok(job) => jobs.push(job),
            Err(error) => tracing::error!(id = %record.id, %error, "Skipped job record that didn't decode"),
        }
    }
    Ok(jobs)
}

/// Every job of `records`; one that doesn't decode fails the whole list.
fn all_jobs(records: &[SqliteRow]) -> Result<Vec<Job>, StoreError> {
    records
        .iter()
        .map(|row| JobRecord::from_row(row)?.job())
        .collect()
}

impl Store {
    /// Saves `job`, replacing the stored one of the same ID. A new job pins the catalog versions of its inputs; a
    /// finished video is registered in the catalog in the same transaction as the job's state.
    pub async fn save_job(&self, job: &Job) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let existing = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(job.id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        let references = if let Some(row) = existing {
            let mut record = JobRecord::from_row(&row)?;
            let references = record.inputs()?;
            record.update(job)?;
            record.write(&mut *connection).await?;
            references
        } else {
            let references = inputs_of(&mut *connection, job).await?;
            JobRecord::new(job, &references)?.insert(&mut *connection).await?;
            references
        };
        register_video(&mut *connection, job, references).await?;
        write.commit(&[StoreChange::Jobs, StoreChange::Materials]).await
    }

    /// The stored job of `id`.
    pub async fn job(&self, id: Uuid) -> Result<Option<Job>, StoreError> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| JobRecord::from_row(&row)?.job()).transpose()
    }

    /// Jobs that are not terminal, oldest first. Records that don't decode (e.g. written by a newer build) are
    /// logged and skipped, so one of them can't stop the others from resuming; they stay in the store.
    pub async fn active_jobs(&self) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query("SELECT * FROM jobs WHERE is_terminal = 0 ORDER BY created_at ASC")
            .fetch_all(self.pool())
            .await?;
        decoded_jobs(&rows)
    }

    /// IDs of every job that is not terminal, records that don't decode included.
    pub async fn active_job_ids(&self) -> Result<HashSet<Uuid>, StoreError> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM jobs WHERE is_terminal = 0")
            .fetch_all(self.pool())
            .await?;
        ids.iter().map(|id| uuid(TABLE, id)).collect()
    }

    /// Unfinished jobs and jobs finished at or after `since`: a superset of what the daily spend cap counts.
    /// Every record must decode: a partial sum must never pass the daily spend cap.
    pub async fn jobs_unfinished_or_finished(&self, since: OffsetDateTime) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query(
            "SELECT * FROM jobs WHERE is_terminal = 0 OR (finished_at IS NOT NULL AND finished_at >= ?)",
        )
        .bind(nanos(since))
        .fetch_all(self.pool())
        .await?;
        all_jobs(&rows)
    }

    /// What the daily spend cap counts on `day` in `unit` (`Budget::committed`); `offset` stands in for the
    /// calendar's time zone.
    /// - `excluding`: a job left out of the sum, e.g. the one whose own price is being checked.
    pub async fn committed_spend(
        &self,
        day: OffsetDateTime,
        unit: CostUnit,
        excluding: Option<Uuid>,
        offset: UtcOffset,
    ) -> Result<Decimal, StoreError> {
        let start_of_day = day.to_offset(offset).replace_time(Time::MIDNIGHT);
        let jobs: Vec<Job> = self
            .jobs_unfinished_or_finished(start_of_day)
            .await?
            .into_iter()
            .filter(|job| Some(job.id) != excluding)
            .collect();
        Ok(Budget::committed(&jobs, day, unit, offset))
    }

    /// Jobs in `kind`, newest first, at most `limit` when given. Records that don't decode are skipped.
    pub async fn jobs_in_state(
        &self,
        kind: JobStateKind,
        limit: Option<u32>,
    ) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query("SELECT * FROM jobs WHERE state_kind = ? ORDER BY created_at DESC LIMIT ?")
            .bind(kind.raw_value())
            .bind(limit.map_or(-1, i64::from))
            .fetch_all(self.pool())
            .await?;
        decoded_jobs(&rows)
    }

    /// The newest `limit` done jobs, for the typical generation time. Records that don't decode are skipped.
    pub async fn done_jobs(&self, limit: u32) -> Result<Vec<Job>, StoreError> {
        self.jobs_in_state(JobStateKind::Done, Some(limit)).await
    }

    /// Terminal jobs (done, failed, rejected, canceled), newest first. Records that don't decode are skipped.
    pub async fn finished_jobs(&self) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query("SELECT * FROM jobs WHERE is_terminal = 1 ORDER BY created_at DESC")
            .fetch_all(self.pool())
            .await?;
        decoded_jobs(&rows)
    }

    /// Terminal jobs finished at or after `since`, for Spending. Records that don't decode are skipped.
    pub async fn finished_jobs_since(&self, since: OffsetDateTime) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query(
            "SELECT * FROM jobs WHERE is_terminal = 1 AND finished_at IS NOT NULL AND finished_at >= ?",
        )
        .bind(nanos(since))
        .fetch_all(self.pool())
        .await?;
        decoded_jobs(&rows)
    }

    /// Failed, rejected and canceled jobs finished after `finished_after` (all of them for `None`), newest
    /// finished first, at most `limit`: the Queue's "Didn't finish" section. Records that don't decode are skipped.
    pub async fn problem_jobs(
        &self,
        finished_after: Option<OffsetDateTime>,
        limit: u32,
    ) -> Result<Vec<Job>, StoreError> {
        let rows = sqlx::query(
            "SELECT * FROM jobs WHERE is_terminal = 1 AND state_kind != ? AND finished_at IS NOT NULL \
             AND finished_at > ? ORDER BY finished_at DESC LIMIT ?",
        )
        .bind(JobStateKind::Done.raw_value())
        .bind(finished_after.map_or(i64::MIN, nanos))
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await?;
        decoded_jobs(&rows)
    }

    /// Removes the job of `id`; nothing when it isn't stored.
    pub async fn delete_job(&self, id: Uuid) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *write.transaction)
            .await?;
        write.commit(&[StoreChange::Jobs]).await
    }
}

#[async_trait]
impl JobRepository for Store {
    async fn save(&self, job: &Job) -> Result<(), PortError> {
        Ok(self.save_job(job).await?)
    }

    async fn job(&self, id: Uuid) -> Result<Option<Job>, PortError> {
        Ok(Self::job(self, id).await?)
    }

    async fn active_jobs(&self) -> Result<Vec<Job>, PortError> {
        Ok(Self::active_jobs(self).await?)
    }

    async fn active_job_ids(&self) -> Result<HashSet<Uuid>, PortError> {
        Ok(Self::active_job_ids(self).await?)
    }

    async fn delete(&self, id: Uuid) -> Result<(), PortError> {
        Ok(self.delete_job(id).await?)
    }
}
