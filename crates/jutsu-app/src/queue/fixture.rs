//! `JobFixture` and `TimingFixture` of the macOS tests.

use std::str::FromStr;

use jutsu_core::{
    ClosedRange, Cost, GenerationEstimate, Job, JobSpec, JobState, Mode, PixelSize, ProviderKind, Resolution,
    SourceClip, VideoModel,
};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

pub fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_000_000).unwrap()
}

pub fn minutes_ago(minutes: f64) -> OffsetDateTime {
    now() - Duration::seconds_f64(minutes * 60.0)
}

pub fn usd(text: &str) -> Cost {
    Cost::usd(Decimal::from_str(text).unwrap(), false)
}

/// A job's fields the tests vary.
pub struct JobFixture {
    pub mode: Mode,
    pub kind: ProviderKind,
    pub cost: Cost,
    pub created_minutes_ago: f64,
    pub finished_minutes_ago: Option<f64>,
    pub progress: f64,
}

impl Default for JobFixture {
    fn default() -> Self {
        Self {
            mode: Mode::MotionTransfer,
            kind: ProviderKind::ApiKey,
            cost: usd("8.168"),
            created_minutes_ago: 3.0,
            finished_minutes_ago: None,
            progress: 0.0,
        }
    }
}

impl JobFixture {
    pub fn job(self, state: JobState) -> Job {
        let spec = JobSpec::new(
            self.mode,
            VideoModel::GENJUTSU,
            Some(SourceClip::new(
                Url::parse("file:///clips/dance.mov").unwrap(),
                30.0,
                PixelSize {
                    width: 1280,
                    height: 720,
                },
                ClosedRange::new(0.0, 12.0),
            )),
            Resolution::P720,
        );
        let mut job = Job::new(
            Uuid::new_v4(),
            spec,
            self.kind,
            self.cost,
            Uuid::new_v4(),
            minutes_ago(self.created_minutes_ago),
        );
        job.state = state;
        job.progress = self.progress;
        job.finished_at = self.finished_minutes_ago.map(minutes_ago);
        job
    }
}

/// `JobFixture::default().job(state)`.
pub fn job(state: JobState) -> Job {
    JobFixture::default().job(state)
}

/// A job created `created` minutes ago that finished `finished` minutes ago.
pub fn finished(state: JobState, created: f64, finished: f64) -> Job {
    JobFixture {
        created_minutes_ago: created,
        finished_minutes_ago: Some(finished),
        ..JobFixture::default()
    }
    .job(state)
}

/// A job with the API timings filled in: submitted `submitted_minutes_ago`, and processed for `processing` seconds
/// after a 10 s queue wait when given.
pub fn timed(state: JobState, submitted_minutes_ago: f64, processing: Option<f64>) -> Job {
    let mut job = JobFixture {
        created_minutes_ago: submitted_minutes_ago + 1.0,
        ..JobFixture::default()
    }
    .job(state);
    let submitted_at = minutes_ago(submitted_minutes_ago);
    job.submitted_at = Some(submitted_at);
    if let Some(processing) = processing {
        job.processing_at = Some(submitted_at + Duration::seconds(10));
        job.charged_at = Some(submitted_at + Duration::seconds_f64(10.0 + processing));
    }
    job
}

pub fn history(processing: f64, count: usize) -> Vec<Job> {
    (0..count)
        .map(|_| timed(JobState::Done, 60.0, Some(processing)))
        .collect()
}

pub const TWO_MINUTES: GenerationEstimate = GenerationEstimate {
    queue_wait: 20.0,
    processing: 100.0,
    sample_count: 4,
};
