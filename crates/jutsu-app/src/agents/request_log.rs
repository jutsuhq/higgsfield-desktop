//! The `request_id`s agents sent their paid requests with, and what each made (`AgentRequestLog.swift`): what makes
//! the same request sent again return its job instead of paying for another. Written before the job is queued, with
//! the job's ID chosen first, so a crash in between still finds the job (or queues that same ID). Read strictly: a
//! file that can't be read refuses every request rather than forgetting which were made. Records are kept
//! `KEPT_DAYS` days.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::images::{LogFile, LogWriteError};

/// One request as the log keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequestRecord {
    #[serde(rename = "requestID")]
    pub request_id: String,
    pub fingerprint: String,
    /// The fingerprint without `max_cost` (`CreateVideoCommand::parameters`); `None` for other commands and for
    /// requests recorded before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<String>,
    #[serde(rename = "jobIDs")]
    pub job_ids: Vec<Uuid>,
    #[serde(with = "jutsu_core::support::date::rfc3339")]
    pub created_at: OffsetDateTime,
    /// What the IDs are when the app keeps them only in memory: `on_device` for runs on this computer, which `jobs`
    /// then answers as forgotten after a relaunch; `None` for jobs and image requests, which are stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What a request is, by its ID and fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentLookup {
    /// Never seen: queue it.
    New,
    /// The same request again: these are its jobs.
    Known(Vec<Uuid>),
    /// The same parameters with another `max_cost`: these are its jobs, whose hold it may lift.
    OtherMaxCost(Vec<Uuid>),
    /// The ID was used with other parameters.
    Conflict,
}

/// `agent-requests.json` in the app data folder, or in memory for tests.
pub struct AgentRequestLog {
    file: LogFile<AgentRequestRecord>,
    now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    records: Option<Vec<AgentRequestRecord>>,
}

impl std::fmt::Debug for AgentRequestLog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentRequestLog").finish_non_exhaustive()
    }
}

impl AgentRequestLog {
    pub const FILE_NAME: &'static str = "agent-requests.json";
    pub const KEPT_DAYS: i64 = 30;

    /// `None` keeps it in memory, for tests.
    pub fn new(folder: Option<&Path>) -> Self {
        Self::with_clock(folder, Arc::new(OffsetDateTime::now_utc))
    }

    pub fn with_clock(folder: Option<&Path>, now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>) -> Self {
        Self {
            file: LogFile::new(folder, Self::FILE_NAME),
            now,
            records: None,
        }
    }

    pub fn lookup(
        &mut self,
        request_id: &str,
        fingerprint: &str,
        parameters: Option<&str>,
    ) -> Result<AgentLookup, LogWriteError> {
        let records = self.load()?;
        let Some(record) = records.iter().find(|record| record.request_id == request_id) else {
            return Ok(AgentLookup::New);
        };
        if record.fingerprint == fingerprint {
            return Ok(AgentLookup::Known(record.job_ids.clone()));
        }
        if let Some(parameters) = parameters
            && record.parameters.as_deref() == Some(parameters)
        {
            return Ok(AgentLookup::OtherMaxCost(record.job_ids.clone()));
        }
        Ok(AgentLookup::Conflict)
    }

    /// The `kind` the request that made `id` was recorded with; `None` for none, or when the log can't be read.
    pub fn kind_of(&mut self, id: Uuid) -> Option<String> {
        self.load()
            .ok()?
            .iter()
            .find(|record| record.job_ids.contains(&id))?
            .kind
            .clone()
    }

    /// The request now stands for `fingerprint`: its job goes on under the `max_cost` it was sent again with.
    pub fn update(&mut self, request_id: &str, fingerprint: &str) -> Result<(), LogWriteError> {
        let mut records = self.load()?;
        let Some(record) = records.iter_mut().find(|record| record.request_id == request_id) else {
            return Ok(());
        };
        fingerprint.clone_into(&mut record.fingerprint);
        self.save(records)
    }

    /// Records a request before its jobs are queued.
    pub fn add(&mut self, record: AgentRequestRecord) -> Result<(), LogWriteError> {
        let mut records = self.load()?;
        records.retain(|old| old.request_id != record.request_id);
        records.push(record);
        self.save(records)
    }

    /// Forgets a request whose job couldn't be queued, so the agent can send it again.
    pub fn remove(&mut self, request_id: &str) -> Result<(), LogWriteError> {
        let mut records = self.load()?;
        records.retain(|record| record.request_id != request_id);
        self.save(records)
    }

    fn load(&mut self) -> Result<Vec<AgentRequestRecord>, LogWriteError> {
        if let Some(records) = &self.records {
            return Ok(records.clone());
        }
        let loaded = self.file.read_strict()?;
        self.records = Some(loaded.clone());
        Ok(loaded)
    }

    fn save(&mut self, updated: Vec<AgentRequestRecord>) -> Result<(), LogWriteError> {
        let cutoff = (self.now)() - Duration::days(Self::KEPT_DAYS);
        let kept: Vec<AgentRequestRecord> = updated
            .into_iter()
            .filter(|record| record.created_at >= cutoff)
            .collect();
        self.file.write(&kept)?;
        self.records = Some(kept);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, jobs: Vec<Uuid>, at: OffsetDateTime) -> AgentRequestRecord {
        AgentRequestRecord {
            request_id: id.to_owned(),
            fingerprint: "f1".to_owned(),
            parameters: None,
            job_ids: jobs,
            created_at: at,
            kind: None,
        }
    }

    #[test]
    fn a_request_is_new_known_or_a_conflict_and_survives_a_relaunch() {
        let folder = tempfile::tempdir().unwrap();
        let job = Uuid::new_v4();
        let mut log = AgentRequestLog::new(Some(folder.path()));
        assert_eq!(log.lookup("r1", "f1", None).unwrap(), AgentLookup::New);
        log.add(record("r1", vec![job], OffsetDateTime::now_utc()))
            .unwrap();

        let mut reopened = AgentRequestLog::new(Some(folder.path()));
        assert_eq!(
            reopened.lookup("r1", "f1", None).unwrap(),
            AgentLookup::Known(vec![job])
        );
        assert_eq!(reopened.lookup("r1", "f2", None).unwrap(), AgentLookup::Conflict);
        reopened.remove("r1").unwrap();
        assert_eq!(
            AgentRequestLog::new(Some(folder.path()))
                .lookup("r1", "f2", None)
                .unwrap(),
            AgentLookup::New
        );
    }

    /// An unreadable record refuses requests rather than forgetting which were made, which could pay twice.
    #[test]
    fn an_unreadable_file_refuses_every_request() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join(AgentRequestLog::FILE_NAME), "{").unwrap();
        let mut log = AgentRequestLog::new(Some(folder.path()));
        assert!(log.lookup("r1", "f1", None).is_err());
        assert!(
            log.add(record("r1", vec![Uuid::new_v4()], OffsetDateTime::now_utc()))
                .is_err()
        );
    }

    #[test]
    fn old_requests_are_forgotten() {
        let mut log = AgentRequestLog::new(None);
        let now = OffsetDateTime::now_utc();
        log.add(record("old", vec![Uuid::new_v4()], now - Duration::days(40)))
            .unwrap();
        log.add(record("new", vec![Uuid::new_v4()], now)).unwrap();
        assert_eq!(log.lookup("old", "f1", None).unwrap(), AgentLookup::New);
        assert_ne!(log.lookup("new", "f1", None).unwrap(), AgentLookup::New);
    }

    /// Records written before parameters existed still read.
    #[test]
    fn a_record_without_parameters_decodes() {
        let json = r#"{"requestID":"r1","fingerprint":"f1","jobIDs":[],"createdAt":"2026-11-01T10:00:00Z"}"#;
        let record: AgentRequestRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.parameters, None);
        assert_eq!(record.fingerprint, "f1");
    }
}
