//! The accepted jobs of image requests still being followed (`ImageJobLog.swift`), in `image-jobs.json` next to the
//! store, so a job that may be paid for is followed again after a quit, a lost sign-in or a switch of account.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use jutsu_core::{ImageRequest, LocalImage, ProviderKind, RemoteJob};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::log_file::{LogFile, LogWriteError};

/// Where a job of a request is, coded as Swift codes the enum: `{"failed":{"_0":"…"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageJobState {
    Pending {},
    Completed {},
    Failed {
        #[serde(rename = "_0", default)]
        message: Option<String>,
    },
    Rejected {},
    Canceled {},
}

/// One job of a request the provider accepted: kept from the moment its submit was answered until the request ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageJobRecord {
    /// The provider's job, with where to poll it.
    pub job: RemoteJob,
    /// `ImageProvider::owner` of the connection that started it.
    #[serde(default)]
    pub owner: Option<String>,
    /// The request it came from, which is also its `ImageCharge`'s ID.
    #[serde(rename = "runID")]
    pub run_id: Uuid,
    #[serde(with = "jutsu_core::support::date::rfc3339")]
    pub created_at: OffsetDateTime,
    pub prompt: String,
    /// The model's title, as the images and the run row show it.
    pub model: String,
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    /// Images the request makes.
    pub count: i64,
    pub state: ImageJobState,
    #[serde(default = "api_key")]
    pub provider_kind: ProviderKind,
    /// An agent's name for the images; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<ImageRequest>,
    #[serde(default)]
    pub input_images: Vec<LocalImage>,
}

const fn api_key() -> ProviderKind {
    ProviderKind::ApiKey
}

impl ImageJobRecord {
    /// The provider's request ID.
    pub fn id(&self) -> &str {
        &self.job.request_id
    }
}

/// The records, oldest first. A write that fails keeps the records in memory, so their jobs are still followed in
/// this launch, says why in `save_error` and lists the job IDs only this launch knows in `unsaved_ids`.
#[derive(Debug)]
pub struct ImageJobLog {
    file: LogFile<ImageJobRecord>,
    state: Mutex<JobLogState>,
}

#[derive(Debug, Default)]
struct JobLogState {
    records: Vec<ImageJobRecord>,
    save_error: Option<LogWriteError>,
    unsaved_ids: Vec<String>,
}

impl ImageJobLog {
    pub const FILE_NAME: &'static str = "image-jobs.json";

    /// `None` keeps everything in memory.
    pub fn open(folder: Option<&Path>) -> Self {
        let file = LogFile::new(folder, Self::FILE_NAME);
        let records = file.read();
        Self {
            file,
            state: Mutex::new(JobLogState {
                records,
                ..JobLogState::default()
            }),
        }
    }

    fn locked(&self) -> MutexGuard<'_, JobLogState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn records(&self) -> Vec<ImageJobRecord> {
        self.locked().records.clone()
    }

    pub fn save_error(&self) -> Option<LogWriteError> {
        self.locked().save_error.clone()
    }

    pub fn unsaved_ids(&self) -> Vec<String> {
        self.locked().unsaved_ids.clone()
    }

    /// The jobs of `owner` still running.
    pub fn pending(&self, owner: Option<&str>) -> Vec<ImageJobRecord> {
        self.locked()
            .records
            .iter()
            .filter(|record| record.owner.as_deref() == owner && record.state == ImageJobState::Pending {})
            .cloned()
            .collect()
    }

    pub fn add(&self, new: Vec<ImageJobRecord>) {
        let mut state = self.locked();
        for record in new {
            if state.records.iter().any(|old| old.id() == record.id()) {
                continue;
            }
            state.unsaved_ids.push(record.id().to_owned());
            state.records.push(record);
        }
        self.save(&mut state);
    }

    pub fn update(&self, id: &str, new: ImageJobState) {
        let mut state = self.locked();
        let Some(record) = state.records.iter_mut().find(|record| record.id() == id) else {
            return;
        };
        record.state = new;
        self.save(&mut state);
    }

    /// The request ended and its outcome is settled: its jobs needn't be followed again.
    pub fn forget(&self, run: Uuid) {
        let mut state = self.locked();
        if !state.records.iter().any(|record| record.run_id == run) {
            return;
        }
        state.records.retain(|record| record.run_id != run);
        self.save(&mut state);
    }

    pub fn retry_save(&self) {
        let mut state = self.locked();
        self.save(&mut state);
    }

    fn save(&self, state: &mut JobLogState) {
        match self.file.write(&state.records) {
            Ok(()) => {
                state.save_error = None;
                state.unsaved_ids.clear();
            }
            Err(error) => state.save_error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, run: Uuid, owner: &str) -> ImageJobRecord {
        ImageJobRecord {
            job: RemoteJob::new(id),
            owner: Some(owner.to_owned()),
            run_id: run,
            created_at: OffsetDateTime::from_unix_timestamp(1_760_310_000).unwrap(),
            prompt: "a knight".to_owned(),
            model: "Soul 2".to_owned(),
            aspect_ratio: None,
            count: 1,
            state: ImageJobState::Pending {},
            provider_kind: ProviderKind::ApiKey,
            title: None,
            request: None,
            input_images: Vec::new(),
        }
    }

    #[test]
    fn records_survive_a_relaunch_until_their_run_is_forgotten() {
        let folder = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let log = ImageJobLog::open(Some(folder.path()));
        log.add(vec![record("req-1", run, "key-1"), record("req-2", run, "key-2")]);
        log.update(
            "req-2",
            ImageJobState::Failed {
                message: Some("Bad".to_owned()),
            },
        );
        let reopened = ImageJobLog::open(Some(folder.path()));
        assert_eq!(
            reopened
                .pending(Some("key-1"))
                .iter()
                .map(ImageJobRecord::id)
                .collect::<Vec<_>>(),
            vec!["req-1"]
        );
        assert_eq!(
            reopened.records()[1].state,
            ImageJobState::Failed {
                message: Some("Bad".to_owned())
            }
        );
        reopened.forget(run);
        assert_eq!(ImageJobLog::open(Some(folder.path())).records().len(), 0);
    }

    /// The accepted jobs stay followed and listed when the file can't be written.
    #[test]
    fn a_failed_write_keeps_the_jobs_and_lists_them_to_copy() {
        let folder = tempfile::tempdir().unwrap();
        let blocker = folder.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let jobs = ImageJobLog::open(Some(&blocker));
        jobs.add(vec![record("req-1", Uuid::new_v4(), "key-1")]);
        assert_eq!(
            jobs.pending(Some("key-1"))
                .iter()
                .map(ImageJobRecord::id)
                .collect::<Vec<_>>(),
            vec!["req-1"]
        );
        assert_eq!(jobs.unsaved_ids(), vec!["req-1".to_owned()]);
        assert_eq!(
            jobs.save_error().map(|error| error.file),
            Some(ImageJobLog::FILE_NAME.to_owned())
        );
    }

    #[test]
    fn states_are_coded_as_swift_codes_them() {
        let text = serde_json::to_string(&ImageJobState::Failed {
            message: Some("x".to_owned()),
        })
        .unwrap();
        assert_eq!(text, r#"{"failed":{"_0":"x"}}"#);
        assert_eq!(
            serde_json::from_str::<ImageJobState>(r#"{"pending":{}}"#).unwrap(),
            ImageJobState::Pending {}
        );
    }
}
