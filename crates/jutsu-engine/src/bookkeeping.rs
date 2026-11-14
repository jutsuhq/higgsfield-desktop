//! The engine's bookkeeping: plain structs only the engine changes, under its state's lock. Each waiter is a
//! oneshot sender the waiting task's receiver stands for, as a Swift continuation does.

use std::collections::{HashMap, HashSet};

use jutsu_core::RemoteStatus;
use time::{Duration, OffsetDateTime};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::clock::Random;
use crate::settings::{EngineSettings, Waits};

/// The engine's submits in flight, as quitting sees them (`JobEngine::stop`): the jobs whose submit is being sent or
/// waits for its answer, whether the engine is stopping so that no new one starts, and the `stop` waiting for the
/// ones out to be answered.
#[derive(Debug, Default)]
pub(crate) struct SubmitTracker {
    /// Jobs whose submit is being sent or waits for its answer.
    pub(crate) in_flight: HashSet<Uuid>,
    /// Set while `stop` gives submits in flight their time; `begin` refuses meanwhile.
    pub(crate) is_stopping: bool,
    waiters: Vec<oneshot::Sender<()>>,
}

impl SubmitTracker {
    #[cfg(test)]
    pub(crate) fn waiter_count(&self) -> usize {
        self.waiters.len()
    }

    /// Records that job `id` sends its submit; `false`, recording nothing, while stopping.
    pub(crate) fn begin(&mut self, id: Uuid) -> bool {
        if self.is_stopping {
            return false;
        }
        self.in_flight.insert(id);
        true
    }

    /// Ends the submit of job `id` started with `begin`; once none is out, the waiters are resumed.
    pub(crate) fn finish(&mut self, id: Uuid) {
        self.in_flight.remove(&id);
        if self.in_flight.is_empty() {
            self.release_waiters();
        }
    }

    /// `waiter` is resumed once no submit is out (at once when none is), or by `release_waiters`.
    pub(crate) fn wait(&mut self, waiter: oneshot::Sender<()>) {
        if self.in_flight.is_empty() {
            let _ = waiter.send(());
            return;
        }
        self.waiters.push(waiter);
    }

    /// Resumes every waiter, as when their time is up. Returns whether any was waiting.
    pub(crate) fn release_waiters(&mut self) -> bool {
        if self.waiters.is_empty() {
            return false;
        }
        for waiter in self.waiters.drain(..) {
            let _ = waiter.send(());
        }
        true
    }
}

/// What the shared status call said about one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchedStatus {
    Status(RemoteStatus),
    /// No answer for this job this time; the job asks `status` for itself at once.
    Alone,
}

/// The jobs waiting for the next shared status call, and the ticker task that makes it. Each waiter is resumed
/// once: with its answer, or dropped (cancelled) when its task stops waiting.
#[derive(Default)]
pub(crate) struct StatusBatch {
    waiters: HashMap<Uuid, oneshot::Sender<BatchedStatus>>,
    /// Makes the shared calls while jobs poll through them; `None` when none runs.
    pub(crate) ticker: Option<tokio_util::sync::CancellationToken>,
}

impl StatusBatch {
    pub(crate) fn is_empty(&self) -> bool {
        self.waiters.is_empty()
    }

    /// The jobs waiting now, for the next call.
    pub(crate) fn waiting(&self) -> Vec<Uuid> {
        self.waiters.keys().copied().collect()
    }

    /// Job `id` waits for the next call.
    pub(crate) fn wait(&mut self, id: Uuid, waiter: oneshot::Sender<BatchedStatus>) {
        self.waiters.insert(id, waiter);
    }

    /// Job `id` stops waiting, as its task was cancelled.
    pub(crate) fn drop_waiter(&mut self, id: Uuid) {
        self.waiters.remove(&id);
    }

    /// Hands each of `ids` still waiting its entry of `answers`, or `Alone` when the call had none for it.
    pub(crate) fn answer(&mut self, ids: &[Uuid], answers: &HashMap<Uuid, BatchedStatus>) {
        for id in ids {
            if let Some(waiter) = self.waiters.remove(id) {
                let _ = waiter.send(answers.get(id).cloned().unwrap_or(BatchedStatus::Alone));
            }
        }
    }

    /// `items` in order, in groups of at most `size` (at least 1).
    pub(crate) fn groups<T: Clone>(items: &[T], size: usize) -> Vec<Vec<T>> {
        items.chunks(size.max(1)).map(<[T]>::to_vec).collect()
    }
}

/// How long the ticker waits before the next shared status call: as long as the provider asked
/// (`BatchStatus::poll_after`, kept in `server_poll_delay`) or, when it didn't say, as a job polled alone would:
/// Higgsfield's backoff, reset whenever a job's status changes.
pub(crate) struct StatusPace {
    backoff: Waits,
    server_delay: (Duration, Duration),
    /// Each answered job's last status, to see a change; a job left out of an answer is forgotten.
    last: HashMap<Uuid, RemoteStatus>,
    /// The wait before the next call.
    pub(crate) delay: Duration,
}

impl StatusPace {
    pub(crate) fn new(settings: &EngineSettings, random: Random) -> Self {
        let mut backoff = Waits::polling(settings, random);
        let delay = backoff.next();
        Self {
            backoff,
            server_delay: settings.server_poll_delay,
            last: HashMap::new(),
            delay,
        }
    }

    /// Takes in one round of calls: what they answered and the shortest wait they asked for.
    pub(crate) fn update(&mut self, answers: &HashMap<Uuid, BatchedStatus>, poll_after: Option<Duration>) {
        let mut changed = false;
        for (id, answer) in answers {
            let BatchedStatus::Status(status) = answer else {
                continue;
            };
            changed = changed || self.last.get(id) != Some(status);
            self.last.insert(*id, status.clone());
        }
        self.last.retain(|id, _| answers.contains_key(id));
        if changed {
            self.backoff.reset();
        }
        self.delay = match poll_after {
            Some(asked) => asked.clamp(self.server_delay.0, self.server_delay.1),
            None => self.backoff.next(),
        };
    }
}

/// Whose turn it is to save a change of each job (`JobEngine::apply`, `save`): one writer per job at a time, so each
/// change starts from the last saved one.
#[derive(Debug, Default)]
pub(crate) struct WriteTurns {
    writers: HashSet<Uuid>,
    waiters: HashMap<Uuid, Vec<oneshot::Sender<()>>>,
}

impl WriteTurns {
    /// Takes job `id`'s turn; `false` while another writer has it.
    pub(crate) fn take(&mut self, id: Uuid) -> bool {
        self.writers.insert(id)
    }

    /// `waiter` is resumed when job `id`'s turn ends; it then tries `take` again.
    pub(crate) fn wait(&mut self, id: Uuid, waiter: oneshot::Sender<()>) {
        self.waiters.entry(id).or_default().push(waiter);
    }

    /// Ends job `id`'s turn and resumes its waiters.
    pub(crate) fn end(&mut self, id: Uuid) {
        self.writers.remove(&id);
        for waiter in self.waiters.remove(&id).unwrap_or_default() {
            let _ = waiter.send(());
        }
    }

    #[cfg(test)]
    pub(crate) fn has_waiters(&self, id: Uuid) -> bool {
        self.waiters.get(&id).is_some_and(|waiters| !waiters.is_empty())
    }
}

/// The tasks waiting for connectivity or the connection to change (`JobEngine::network_provider`).
#[derive(Debug, Default)]
pub(crate) struct NetworkGate {
    waiters: HashMap<Uuid, oneshot::Sender<()>>,
}

impl NetworkGate {
    /// `waiter`, known by `token`, is resumed by the next `open`.
    pub(crate) fn wait(&mut self, token: Uuid, waiter: oneshot::Sender<()>) {
        self.waiters.insert(token, waiter);
    }

    /// Resumes every waiter; each checks again what it waits for.
    pub(crate) fn open(&mut self) {
        for (_, waiter) in self.waiters.drain() {
            let _ = waiter.send(());
        }
    }

    /// Resumes the waiter of `token` alone, as its task was cancelled.
    pub(crate) fn open_one(&mut self, token: Uuid) {
        if let Some(waiter) = self.waiters.remove(&token) {
            let _ = waiter.send(());
        }
    }
}

/// Decides which progress reports reach the engine's events. A re-encode reports per sample, hundreds of times a
/// second. Only progress is coalesced: every other change is published at once and resets the baseline.
#[derive(Debug)]
pub(crate) struct ProgressCoalescer {
    /// Smallest change of the fraction worth publishing.
    pub(crate) min_step: f64,
    /// After this any change is published.
    pub(crate) max_interval: Duration,
    published: HashMap<Uuid, (f64, OffsetDateTime)>,
}

impl Default for ProgressCoalescer {
    fn default() -> Self {
        Self {
            min_step: 0.01,
            max_interval: Duration::milliseconds(250),
            published: HashMap::new(),
        }
    }
}

impl ProgressCoalescer {
    /// Records an event the engine published for `id`.
    pub(crate) fn did_publish(&mut self, id: Uuid, progress: f64, at: OffsetDateTime) {
        self.published.insert(id, (progress, at));
    }

    pub(crate) fn forget(&mut self, id: Uuid) {
        self.published.remove(&id);
    }

    /// Whether progress reported for `id` should be published now. Start and end always are.
    pub(crate) fn should_publish(&self, id: Uuid, progress: f64, at: OffsetDateTime) -> bool {
        if progress <= 0.0 || progress >= 1.0 {
            return true;
        }
        let Some((last, published_at)) = self.published.get(&id) else {
            return true;
        };
        #[allow(clippy::float_cmp, reason = "the same report again is the same value")]
        if progress == *last {
            return false;
        }
        (progress - last).abs() >= self.min_step || at - *published_at >= self.max_interval
    }
}
