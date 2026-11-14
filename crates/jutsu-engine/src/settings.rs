//! What the engine is set to, and the waits made from it.

use std::path::PathBuf;

use jutsu_core::{Backoff, OutputNamePattern};
use time::{Duration, UtcOffset};

use crate::clock::Random;

/// The engine's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineSettings {
    /// Finished videos land here: the app passes the `Videos` folder of the output folder (`OutputSubfolder`).
    pub output_directory: PathBuf,
    /// Names each result when it is placed in `output_directory`.
    pub output_name_pattern: OutputNamePattern,
    /// The time zone results are dated in (`{date}`); the Mac's calendar in Swift. UTC until the app sets the local one.
    pub utc_offset: UtcOffset,
    /// How many jobs may be submitted or running at once.
    pub max_concurrency: usize,
    /// Wait before the first poll and after every status change. Higgsfield's polling guide: start at 2 s, grow
    /// by 1.5× up to 10 s.
    pub poll_initial_delay: Duration,
    /// The poll wait grows by 1.5× up to this.
    pub poll_max_delay: Duration,
    /// The wait a provider asks for between shared status calls (`BatchStatus::poll_after`) is kept in this range,
    /// so a server asking for 0 isn't hammered and one asking for an hour doesn't stall its jobs.
    pub server_poll_delay: (Duration, Duration),
    /// Wait before the first retry of a failed step (network, 5xx, concurrency limit).
    pub retry_initial_delay: Duration,
    /// The retry wait doubles up to this; longer than polling, so an outage isn't hammered.
    pub retry_max_delay: Duration,
    /// The wait a provider asks for before a refused step is sent again (`Retry-After` of a full queue) is kept in
    /// this range, so a server asking for 0 isn't hammered and one asking for a day doesn't stall the job.
    pub server_retry_delay: (Duration, Duration),
    /// A job not yet submitted whose step has failed this long, and at least `retry_pause_failures` times in a
    /// row, pauses (`KeptFailing`) instead of retrying on. Both, so a computer that slept through one wait doesn't
    /// pause a job on its next failure.
    pub retry_pause_after: Duration,
    /// See `retry_pause_after`.
    pub retry_pause_failures: i64,
    /// Every wait is scaled by a random factor in `1 ± jitter`.
    pub jitter: f64,
    /// Older uploads are uploaded again: the API doesn't document how long it keeps them.
    pub upload_max_age: Duration,
    /// A submitted job still unfinished this long after submit fails.
    pub generation_deadline: Duration,
    /// Larger downloads are refused, in bytes.
    pub max_download_bytes: u64,
}

impl EngineSettings {
    /// The defaults of the macOS app, with results in `output_directory`.
    pub fn new(output_directory: PathBuf) -> Self {
        Self {
            output_directory,
            output_name_pattern: OutputNamePattern::standard(),
            utc_offset: UtcOffset::UTC,
            max_concurrency: 2,
            poll_initial_delay: Duration::seconds(2),
            poll_max_delay: Duration::seconds(10),
            server_poll_delay: (Duration::seconds(1), Duration::seconds(30)),
            retry_initial_delay: Duration::seconds(5),
            retry_max_delay: Duration::seconds(60),
            server_retry_delay: (Duration::seconds(1), Duration::seconds(600)),
            retry_pause_after: Duration::minutes(15),
            retry_pause_failures: 5,
            jitter: 0.2,
            upload_max_age: Duration::hours(1),
            generation_deadline: Duration::hours(2),
            max_download_bytes: 4 << 30,
        }
    }
}

/// A backoff whose every wait is scaled by a random factor in `1 ± jitter`, the randomness injected.
#[derive(Clone)]
pub(crate) struct Waits {
    backoff: Backoff,
    jitter: f64,
    random: Random,
}

impl Waits {
    /// Higgsfield's polling guide, from the settings.
    pub(crate) fn polling(settings: &EngineSettings, random: Random) -> Self {
        Self {
            backoff: Backoff::polling_with(settings.poll_initial_delay, settings.poll_max_delay, 0.0),
            jitter: settings.jitter.clamp(0.0, 1.0),
            random,
        }
    }

    /// The retries of a failed step: doubling up to the cap.
    pub(crate) fn retrying(settings: &EngineSettings, random: Random) -> Self {
        Self {
            backoff: Backoff::new(settings.retry_initial_delay, settings.retry_max_delay, 2.0, 0.0),
            jitter: settings.jitter.clamp(0.0, 1.0),
            random,
        }
    }

    /// The next wait.
    pub(crate) fn next(&mut self) -> Duration {
        let base = self.backoff.next_wait();
        if self.jitter == 0.0 {
            return base;
        }
        base * (1.0 + self.jitter * ((self.random)() * 2.0 - 1.0))
    }

    /// Back to the first wait.
    pub(crate) fn reset(&mut self) {
        self.backoff.reset();
    }
}
