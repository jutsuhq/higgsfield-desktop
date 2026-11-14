//! How long the API took to generate past jobs.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::models::{Job, JobSpec, Mode, Resolution, VideoModel};
use crate::rules::Pricing;
use crate::support::Seconds;

/// What makes two jobs comparable. Billed length is bucketed rather than scaled per second: generation time has a
/// large fixed part, so scaling a 5 s job to a 60 s one would be far off.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenerationTimesKey {
    /// The model.
    pub model: VideoModel,
    /// The mode.
    pub mode: Mode,
    /// The resolution.
    pub resolution: Resolution,
    /// The upper bound of the billed length bucket, in seconds; `i64::MAX` for the longest one.
    pub length_bucket: i64,
}

impl GenerationTimesKey {
    /// The key of `spec`.
    pub fn new(spec: &JobSpec) -> Self {
        Self {
            model: spec.model.clone(),
            mode: spec.mode,
            resolution: spec.resolution,
            length_bucket: GenerationTimes::length_bucket(Pricing::billed_seconds(spec.duration())),
        }
    }
}

/// The typical API side of a job: the Generate step, from submit until the API finished it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenerationEstimate {
    /// Median seconds from submit until the API started generating.
    pub queue_wait: Seconds,
    /// Median seconds from the start of the generation until the API finished it.
    pub processing: Seconds,
    /// How many past jobs the medians come from.
    pub sample_count: usize,
}

impl GenerationEstimate {
    /// Queue wait and processing together.
    pub fn total(&self) -> Seconds {
        self.queue_wait + self.processing
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    queue_wait: Seconds,
    processing: Seconds,
}

impl Sample {
    fn of(job: &Job) -> Option<Self> {
        let (submitted_at, processing_at, processing) = (
            job.submitted_at?,
            job.processing_at?,
            job.processing_time()?.as_seconds_f64(),
        );
        let queue_wait = (processing_at - submitted_at).as_seconds_f64();
        (queue_wait >= 0.0 && processing >= 0.0).then_some(Self {
            queue_wait,
            processing,
        })
    }
}

/// How long the API took to generate the user's past jobs, grouped by what drives that time. The API reports
/// no progress while it generates, so the median of similar jobs is the only sense of time left there is.
#[derive(Debug, Clone, Default)]
pub struct GenerationTimes {
    samples: HashMap<GenerationTimesKey, HashMap<Uuid, Sample>>,
    estimates: HashMap<GenerationTimesKey, GenerationEstimate>,
}

impl GenerationTimes {
    /// Fewer similar jobs than this give no estimate: one slow outlier would be the whole answer.
    pub const MINIMUM_SAMPLES: usize = 3;

    /// Upper bounds of the billed length buckets, in seconds.
    const LENGTH_BUCKETS: [i64; 5] = [5, 10, 15, 30, 60];

    /// The times of `jobs`.
    pub fn new(jobs: &[Job]) -> Self {
        let mut times = Self::default();
        times.record(jobs);
        times
    }

    /// The typical time for `spec`; `None` with fewer than `MINIMUM_SAMPLES` similar jobs.
    pub fn estimate(&self, spec: &JobSpec) -> Option<GenerationEstimate> {
        self.estimates.get(&GenerationTimesKey::new(spec)).copied()
    }

    /// `record` would change the estimates: the job has timings not recorded yet.
    pub fn needs_recording(&self, job: &Job) -> bool {
        let Some(sample) = Sample::of(job) else {
            return false;
        };
        self.samples
            .get(&GenerationTimesKey::new(&job.spec))
            .and_then(|samples| samples.get(&job.id))
            != Some(&sample)
    }

    /// Adds the timings of jobs the API finished; other jobs are ignored, and a job recorded again replaces itself.
    pub fn record(&mut self, jobs: &[Job]) {
        let mut changed: HashSet<GenerationTimesKey> = HashSet::new();
        for job in jobs {
            let Some(sample) = Sample::of(job) else { continue };
            let key = GenerationTimesKey::new(&job.spec);
            self.samples
                .entry(key.clone())
                .or_default()
                .insert(job.id, sample);
            changed.insert(key);
        }
        for key in changed {
            let samples: Vec<Sample> = self
                .samples
                .get(&key)
                .map(|samples| samples.values().copied().collect())
                .unwrap_or_default();
            match Self::estimate_from(&samples) {
                Some(estimate) => self.estimates.insert(key, estimate),
                None => self.estimates.remove(&key),
            };
        }
    }

    fn estimate_from(samples: &[Sample]) -> Option<GenerationEstimate> {
        if samples.len() < Self::MINIMUM_SAMPLES {
            return None;
        }
        let queue_wait = Self::median(&samples.iter().map(|sample| sample.queue_wait).collect::<Vec<_>>())?;
        let processing = Self::median(&samples.iter().map(|sample| sample.processing).collect::<Vec<_>>())?;
        Some(GenerationEstimate {
            queue_wait,
            processing,
            sample_count: samples.len(),
        })
    }

    /// The middle value, or the mean of the two middle ones; `None` for no values.
    pub fn median(values: &[f64]) -> Option<f64> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let middle = sorted.len() / 2;
        Some(if sorted.len().is_multiple_of(2) {
            f64::midpoint(sorted[middle - 1], sorted[middle])
        } else {
            sorted[middle]
        })
    }

    /// The upper bound of the bucket `billed_seconds` falls in.
    pub fn length_bucket(billed_seconds: i64) -> i64 {
        Self::LENGTH_BUCKETS
            .iter()
            .copied()
            .find(|bound| billed_seconds <= *bound)
            .unwrap_or(i64::MAX)
    }
}
