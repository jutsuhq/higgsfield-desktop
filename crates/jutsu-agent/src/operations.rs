//! The exports an agent started (`export`, or `delivery_set` with action export). Each runs in a task of its own, so a
//! call that started it answers at once with its ID, and `jobs` follows it like a job.

use crate::{AgentError, AgentExportFile, AgentExportStatus, AgentJob, Timestamp, Uuid, values::wire_enum};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::watch;

wire_enum! {
    pub enum AgentOperationKind { Export = "export", DeliveryExport = "delivery_export" }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentOperationState {
    Running,
    /// Every file was tried; each says how it went.
    Done,
    /// It stopped before trying them all, for this reason.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentOperation {
    pub id: Uuid,
    pub kind: AgentOperationKind,
    pub state: AgentOperationState,
    /// Where the files go.
    pub folder: String,
    /// How many files it writes, skipped ones included.
    pub total: i64,
    /// The files finished so far, then all of them in the order asked.
    pub files: Vec<AgentExportFile>,
    /// A delivery export's set.
    pub set: Option<Uuid>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

impl AgentOperation {
    /// As `jobs` lists it.
    pub fn job(&self) -> AgentJob {
        let (state, error) = match &self.state {
            AgentOperationState::Running => ("processing", None),
            AgentOperationState::Done => ("done", None),
            AgentOperationState::Failed(message) => ("failed", Some(message.clone())),
        };
        let written: Vec<String> = self
            .files
            .iter()
            .filter(|file| file.status == AgentExportStatus::Done)
            .filter_map(|file| file.file.clone())
            .collect();
        #[allow(clippy::cast_precision_loss, reason = "a count of files")]
        let progress = if self.total == 0 {
            1.0
        } else {
            (self.files.len() as f64 / self.total as f64).min(1.0)
        };
        let mut job = AgentJob::new(self.id, state, Some(self.created_at));
        job.progress = Some(progress);
        job.error = error;
        job.results = (!written.is_empty()).then_some(written);
        self.kind.as_str().clone_into(&mut job.kind);
        job.folder = Some(self.folder.clone());
        job.files = Some(self.files.clone());
        job.set = self.set;
        job.finished_at = self.finished_at;
        job
    }
}

impl AgentJob {
    /// An ID Jutsu Studio has no job, image request or export for, as `jobs` answers it: a relaunch forgets exports.
    pub fn unknown(id: Uuid) -> Self {
        let mut job = Self::new(id, "unknown", None);
        job.error = Some("Jutsu Studio has no job, image request or export with this ID.".to_owned());
        "unknown".clone_into(&mut job.kind);
        job.next = Some(
            "An export's ID is forgotten when Jutsu Studio quits: look for its files in the folder export answered \
             with, or send export again. Call jobs without ids to list the latest work."
                .to_owned(),
        );
        job
    }
}

/// Reports one file as it finishes.
pub type Report = Arc<dyn Fn(AgentExportFile) + Send + Sync>;

type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

struct Entry {
    operation: AgentOperation,
    finished: watch::Receiver<bool>,
}

/// The exports agents started, kept in memory: a relaunch forgets them, and `jobs` then answers `unknown` for their
/// IDs. Finished ones are kept for a day.
#[derive(Clone)]
pub struct AgentOperations {
    entries: Arc<Mutex<HashMap<Uuid, Entry>>>,
    now: Clock,
}

impl Default for AgentOperations {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentOperations {
    /// Seconds a finished export is kept.
    pub const KEPT_FOR: f64 = 86_400.0;

    pub fn new() -> Self {
        Self::with_clock(Arc::new(Timestamp::now))
    }

    pub fn with_clock(now: Arc<dyn Fn() -> Timestamp + Send + Sync>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            now,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts `work` on the tokio runtime and returns the export as it is now, running.
    pub fn start<W, F>(
        &self,
        kind: AgentOperationKind,
        folder: &str,
        total: i64,
        set: Option<Uuid>,
        work: W,
    ) -> AgentOperation
    where
        W: FnOnce(Report) -> F,
        F: Future<Output = Result<Vec<AgentExportFile>, AgentError>> + Send + 'static,
    {
        self.forget_old();
        let operation = AgentOperation {
            id: Uuid::new_v4(),
            kind,
            state: AgentOperationState::Running,
            folder: folder.to_owned(),
            total: total.max(0),
            files: Vec::new(),
            set,
            created_at: (self.now)(),
            finished_at: None,
        };
        let id = operation.id;
        let (done, finished) = watch::channel(false);
        self.lock().insert(
            id,
            Entry {
                operation: operation.clone(),
                finished,
            },
        );
        let reporter = self.clone();
        let report: Report = Arc::new(move |file| {
            if let Some(entry) = reporter.lock().get_mut(&id) {
                entry.operation.files.push(file);
            }
        });
        let future = work(report);
        let owner = self.clone();
        tokio::spawn(async move {
            let result = future.await;
            owner.finish(id, result);
            let _ = done.send(true);
        });
        operation
    }

    pub fn operation(&self, id: Uuid) -> Option<AgentOperation> {
        self.lock().get(&id).map(|entry| entry.operation.clone())
    }

    pub fn all(&self) -> Vec<AgentOperation> {
        self.lock()
            .values()
            .map(|entry| entry.operation.clone())
            .collect()
    }

    /// Returns once `id` has finished; at once for one that isn't running.
    pub async fn finished(&self, id: Uuid) {
        let receiver = self.lock().get(&id).map(|entry| entry.finished.clone());
        if let Some(mut receiver) = receiver {
            let _ = receiver.wait_for(|done| *done).await;
        }
    }

    fn finish(&self, id: Uuid, result: Result<Vec<AgentExportFile>, AgentError>) {
        let finished_at = (self.now)();
        let mut entries = self.lock();
        let Some(entry) = entries.get_mut(&id) else { return };
        match result {
            Ok(files) => {
                entry.operation.files = files;
                entry.operation.state = AgentOperationState::Done;
            }
            Err(error) => entry.operation.state = AgentOperationState::Failed(error.message),
        }
        entry.operation.finished_at = Some(finished_at);
    }

    fn forget_old(&self) {
        let cutoff = (self.now)().adding_seconds(-Self::KEPT_FOR);
        self.lock().retain(|_, entry| {
            entry
                .operation
                .finished_at
                .is_none_or(|finished| finished >= cutoff)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentOperationKind, AgentOperationState, AgentOperations};
    use crate::{AgentError, AgentExportFile, AgentExportStatus, AgentJob, ErrorCode, Timestamp, Uuid};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::Notify;

    const FOLDER: &str = "/tmp/Exports";

    fn file(item: Uuid, status: AgentExportStatus) -> AgentExportFile {
        AgentExportFile {
            item,
            status,
            file: (status == AgentExportStatus::Done).then(|| format!("{FOLDER}/{item}.mov")),
            sidecar: None,
            message: None,
            output: None,
        }
    }

    async fn eventually(mut condition: impl FnMut() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the condition never held");
    }

    /// An export answers running at once; its files show as they finish, and in the order asked once it is done.
    #[tokio::test]
    async fn an_export_runs_on_its_own_and_reports_each_file() {
        let operations = AgentOperations::new();
        let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
        let gate = Arc::new(Notify::new());
        let opened = gate.clone();
        let started = operations.start(
            AgentOperationKind::Export,
            FOLDER,
            2,
            None,
            move |report| async move {
                report(file(second, AgentExportStatus::Done));
                opened.notified().await;
                report(file(first, AgentExportStatus::Failed));
                Ok(vec![
                    file(first, AgentExportStatus::Failed),
                    file(second, AgentExportStatus::Done),
                ])
            },
        );
        assert_eq!(started.state, AgentOperationState::Running);
        assert_eq!(started.job().state, "processing");
        assert_eq!(started.job().kind, "export");
        eventually(|| {
            operations
                .operation(started.id)
                .is_some_and(|operation| operation.files.len() == 1)
        })
        .await;
        assert_eq!(
            operations
                .operation(started.id)
                .and_then(|operation| operation.job().progress),
            Some(0.5)
        );
        gate.notify_one();
        operations.finished(started.id).await;
        let done = operations.operation(started.id).expect("kept");
        assert_eq!(done.state, AgentOperationState::Done);
        assert_eq!(
            done.files.iter().map(|file| file.item).collect::<Vec<_>>(),
            vec![first, second]
        );
        let job = done.job();
        assert_eq!(job.state, "done");
        assert_eq!(job.progress, Some(1.0));
        assert_eq!(job.results, Some(vec![format!("{FOLDER}/{second}.mov")]));
        assert_eq!(job.folder.as_deref(), Some(FOLDER));
        assert!(job.finished_at.is_some());
    }

    #[tokio::test]
    async fn an_export_that_stops_says_why() {
        let operations = AgentOperations::new();
        let set = Uuid::new_v4();
        let started = operations.start(
            AgentOperationKind::DeliveryExport,
            FOLDER,
            3,
            Some(set),
            |_| async {
                Err(AgentError::new(
                    ErrorCode::FAILED,
                    "The disk is full.",
                    true,
                    None,
                ))
            },
        );
        operations.finished(started.id).await;
        let job = operations.operation(started.id).expect("kept").job();
        assert_eq!(job.state, "failed");
        assert_eq!(job.error.as_deref(), Some("The disk is full."));
        assert_eq!(job.kind, "delivery_export");
        assert_eq!(job.set, Some(set));
    }

    /// Finished exports are forgotten after a day; running ones never.
    #[tokio::test]
    async fn finished_exports_are_kept_for_a_day() {
        let clock = Arc::new(Mutex::new(Timestamp::from_unix(1_000_000)));
        let reader = clock.clone();
        let operations = AgentOperations::with_clock(Arc::new(move || *reader.lock().expect("clock")));
        let gate = Arc::new(Notify::new());
        let opened = gate.clone();
        let done = operations.start(AgentOperationKind::Export, FOLDER, 0, None, |_| async {
            Ok(Vec::new())
        });
        let running = operations.start(AgentOperationKind::Export, FOLDER, 1, None, move |_| async move {
            opened.notified().await;
            Ok(Vec::new())
        });
        operations.finished(done.id).await;
        {
            let mut now = clock.lock().expect("clock");
            *now = now.adding_seconds(AgentOperations::KEPT_FOR + 1.0);
        }
        let _ = operations.start(AgentOperationKind::Export, FOLDER, 0, None, |_| async {
            Ok(Vec::new())
        });
        assert!(operations.operation(done.id).is_none());
        assert!(operations.operation(running.id).is_some());
        gate.notify_one();
    }

    /// A handle the app no longer knows, as after a relaunch, answers `unknown` with a next step.
    #[test]
    fn a_forgotten_handle_is_unknown() {
        let id = Uuid::new_v4();
        let job = AgentJob::unknown(id);
        assert_eq!(job.id, id);
        assert_eq!(job.state, "unknown");
        assert!(job.next.is_some_and(|next| !next.is_empty()));
    }
}
