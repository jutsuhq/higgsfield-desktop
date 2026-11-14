//! `GenerationTimesTests.swift`.

use jutsu_core::*;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::common::*;

fn start() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_000_000).unwrap()
}

fn after(seconds: f64) -> OffsetDateTime {
    start() + Duration::seconds_f64(seconds)
}

fn finished(spec: &JobSpec, queue_wait: f64, processing: f64, id: Uuid) -> Job {
    let mut job = Job::new(
        id,
        spec.clone(),
        ProviderKind::ApiKey,
        usd("1", false),
        Uuid::new_v4(),
        start(),
    );
    job.submitted_at = Some(after(5.0));
    job.processing_at = Some(after(5.0 + queue_wait));
    job.charged_at = Some(after(5.0 + queue_wait + processing));
    job.state = JobState::Done;
    job
}

fn processed(spec: &JobSpec, processing: f64) -> Job {
    finished(spec, 10.0, processing, Uuid::new_v4())
}

fn spec(duration: f64) -> JobSpec {
    spec_with(SpecArgs {
        duration,
        ..SpecArgs::default()
    })
}

#[test]
fn median() {
    let cases: [(&[f64], Option<f64>); 5] = [
        (&[], None),
        (&[4.0], Some(4.0)),
        (&[3.0, 1.0, 2.0], Some(2.0)),
        (&[4.0, 1.0, 3.0, 2.0], Some(2.5)),
        (&[5.0, 5.0, 100.0], Some(5.0)),
    ];
    for (values, expected) in cases {
        assert_eq!(GenerationTimes::median(values), expected);
    }
}

#[test]
fn length_buckets() {
    for (billed, bucket) in [
        (0, 5),
        (5, 5),
        (6, 10),
        (12, 15),
        (30, 30),
        (31, 60),
        (61, i64::MAX),
    ] {
        assert_eq!(GenerationTimes::length_bucket(billed), bucket);
    }
}

#[test]
fn no_estimate_with_fewer_than_three_similar_jobs() {
    let spec = spec(10.0);
    let times = GenerationTimes::new(&[processed(&spec, 100.0), processed(&spec, 120.0)]);
    assert_eq!(times.estimate(&spec), None);
}

#[test]
#[allow(clippy::float_cmp, reason = "exact medians, as the Swift test expects")]
fn estimate_is_the_median_of_similar_jobs() {
    let spec = spec(10.0);
    let times = GenerationTimes::new(&[
        finished(&spec, 4.0, 100.0, Uuid::new_v4()),
        finished(&spec, 30.0, 600.0, Uuid::new_v4()),
        finished(&spec, 10.0, 110.0, Uuid::new_v4()),
    ]);
    let estimate = times.estimate(&spec).unwrap();
    assert_eq!(
        estimate,
        GenerationEstimate {
            queue_wait: 10.0,
            processing: 110.0,
            sample_count: 3
        }
    );
    assert_eq!(estimate.total(), 120.0);
}

#[test]
fn only_similar_jobs_count() {
    let spec = spec_with(SpecArgs {
        duration: 10.0,
        resolution: Resolution::P720,
        ..SpecArgs::default()
    });
    let mut other = vec![
        spec_with(SpecArgs {
            duration: 10.0,
            resolution: Resolution::P1080,
            ..SpecArgs::default()
        }),
        spec_with(SpecArgs {
            mode: Mode::Restyle,
            duration: 10.0,
            resolution: Resolution::P720,
            ..SpecArgs::default()
        }),
        spec_with(SpecArgs {
            duration: 20.0,
            resolution: Resolution::P720,
            ..SpecArgs::default()
        }),
    ];
    let mut other_model = spec.clone();
    other_model.model = VideoModel::SEEDANCE25;
    other.push(other_model.clone());
    let mut jobs: Vec<Job> = other
        .iter()
        .flat_map(|spec| (0..3).map(|_| processed(spec, 50.0)))
        .collect();
    jobs.push(processed(&spec, 90.0));
    let times = GenerationTimes::new(&jobs);
    assert_eq!(times.estimate(&spec), None);
    assert_eq!(
        times.estimate(&other_model).map(|estimate| estimate.processing),
        Some(50.0)
    );
}

#[test]
fn lengths_in_one_bucket_share_an_estimate() {
    let times = GenerationTimes::new(&[
        processed(&spec(6.0), 60.0),
        processed(&spec(8.0), 70.0),
        processed(&spec(10.0), 80.0),
    ]);
    assert_eq!(
        times.estimate(&spec(9.0)).map(|estimate| estimate.processing),
        Some(70.0)
    );
    assert_eq!(times.estimate(&spec(11.0)), None);
}

#[test]
fn jobs_without_all_timestamps_are_ignored() {
    let spec = make_spec();
    let mut not_started = processed(&spec, 60.0);
    not_started.processing_at = None;
    let mut not_submitted = processed(&spec, 60.0);
    not_submitted.submitted_at = None;
    let mut not_charged = processed(&spec, 60.0);
    not_charged.charged_at = None;
    let times = GenerationTimes::new(&[
        not_started,
        not_submitted,
        not_charged.clone(),
        processed(&spec, 60.0),
    ]);
    assert_eq!(times.estimate(&spec), None);
    assert!(!times.needs_recording(&not_charged));
}

#[test]
fn recording_a_job_again_replaces_it() {
    let spec = make_spec();
    let id = Uuid::new_v4();
    let mut times = GenerationTimes::new(&[
        processed(&spec, 60.0),
        processed(&spec, 60.0),
        finished(&spec, 10.0, 60.0, id),
    ]);
    let same = finished(&spec, 10.0, 60.0, id);
    assert!(!times.needs_recording(&same));
    let slower = finished(&spec, 10.0, 300.0, id);
    assert!(times.needs_recording(&slower));
    times.record(&[slower]);
    assert_eq!(
        times.estimate(&spec).map(|estimate| estimate.sample_count),
        Some(3)
    );
}
