//! `JobTests.swift`.

use jutsu_core::*;
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::common::*;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap()
}

fn round_trip(job: &Job) -> Job {
    serde_json::from_str(&serde_json::to_string(job).unwrap()).unwrap()
}

fn without(job: &Job, keys: &[&str]) -> Job {
    let mut object = serde_json::to_value(job).unwrap();
    for key in keys {
        assert!(object.as_object_mut().unwrap().remove(*key).is_some(), "{key}");
    }
    serde_json::from_value(object).unwrap()
}

#[test]
fn new_job_starts_preparing_and_round_trips_through_json() {
    let source = SourceClip::new(
        file_url("/tmp/a.mp4"),
        10.0,
        PixelSize::new(1920, 1080),
        ClosedRange::new(0.0, 10.0),
    );
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::genjutsu(),
        Some(source),
        Resolution::P480,
    );
    spec.references = vec![LocalImage::new(file_url("/tmp/a.jpg"))];
    let mut job = new_job(spec, ProviderKind::ApiKey, usd("3.18", false));
    assert_eq!(job.state, JobState::Preparing);
    assert!(job.progress.abs() < f64::EPSILON);

    let mut remote = RemoteJob::new("r1");
    remote.correlation_id = Some("c1".into());
    job.remote = Some(remote);
    job.charged_at = Some(at(1_000));
    assert_eq!(round_trip(&job), job);
}

/// Jobs stored before charges, owners, submit attempts, batches and parents were recorded decode without them.
#[test]
fn job_stored_before_later_fields_decodes_without_them() {
    let mut job = new_job(make_spec(), ProviderKind::Account, credits("9", false));
    job.charged_at = Some(at(1_000));
    job.owner = Some("user_1".into());
    job.submit_attempt = Some(SubmitAttempt { is_idempotent: false });
    job.batch_id = Some(Uuid::new_v4());
    job.parent_id = Some(Uuid::new_v4());

    let decoded = without(
        &job,
        &["chargedAt", "owner", "submitAttempt", "batchID", "parentID"],
    );

    assert_eq!(decoded.charged_at, None);
    assert_eq!(decoded.owner, None);
    assert_eq!(decoded.submit_attempt, None);
    assert_eq!(decoded.batch_id, None);
    assert_eq!(decoded.parent_id, None);
    assert_eq!(decoded.id, job.id);
    assert_eq!(round_trip(&job), job);
}

#[test]
fn remote_job_stored_without_returned_urls_decodes_and_the_submit_status_is_never_stored() {
    let legacy = r#"{"requestID": "r1", "correlationID": "c1"}"#;
    let mut expected_legacy = RemoteJob::new("r1");
    expected_legacy.correlation_id = Some("c1".into());
    assert_eq!(
        serde_json::from_str::<RemoteJob>(legacy).unwrap(),
        expected_legacy
    );

    let mut remote = RemoteJob::new("r1");
    remote.status_url = Some(Url::parse("https://api.example/requests/r1/status").unwrap());
    remote.cancel_url = Some(Url::parse("https://api.example/requests/r1/cancel").unwrap());
    remote.initial_status = Some(RemoteStatus::Queued);
    let decoded: RemoteJob = serde_json::from_str(&serde_json::to_string(&remote).unwrap()).unwrap();
    let mut expected = remote;
    expected.initial_status = None;
    assert_eq!(decoded, expected);
}

#[test]
fn price_is_the_server_quote_once_there_is_one() {
    let mut job = new_job(make_spec(), ProviderKind::ApiKey, usd("3", false));
    assert_eq!(*job.price(), usd("3", false));
    job.server_price = Some(usd("2.5", false));
    assert_eq!(*job.price(), usd("2.5", false));
    assert_eq!(round_trip(&job), job);
}

#[test]
fn only_a_finished_remote_generation_has_a_paid_output() {
    let mut job = new_job(make_spec(), ProviderKind::ApiKey, usd("1", false));
    job.remote = Some(RemoteJob::new("r1"));
    assert!(!job.has_paid_output());
    job.charged_at = Some(at(1_000));
    assert!(job.has_paid_output());
    job.remote = None;
    assert!(!job.has_paid_output());
}

#[test]
fn only_a_job_failed_at_the_deadline_may_still_finish() {
    let mut job = new_job(make_spec(), ProviderKind::ApiKey, usd("1", false));
    job.remote = Some(RemoteJob::new("r1"));
    job.state = JobState::Failed {
        reason: FailureReason::Deadline,
    };
    assert!(job.may_still_finish());
    job.state = JobState::Failed {
        reason: FailureReason::GenerationFailed,
    };
    assert!(!job.may_still_finish());
    job.state = JobState::Failed {
        reason: FailureReason::Message("Higgsfield didn't finish this job in time.".into()),
    };
    assert!(!job.may_still_finish());
}

#[test]
fn job_stored_before_stage_times_decodes_without_them() {
    let mut job = Job::new(
        Uuid::new_v4(),
        make_spec(),
        ProviderKind::ApiKey,
        usd("3.18", false),
        Uuid::new_v4(),
        at(1_000),
    );
    job.submitted_at = Some(at(1_100));
    job.processing_at = Some(at(1_160));
    job.charged_at = Some(at(1_250));
    job.promoted_at = Some(at(1_050));
    assert_eq!(round_trip(&job), job);
    assert_eq!(job.queue_wait(), Some(Duration::seconds(160)));
    assert_eq!(job.processing_time(), Some(Duration::seconds(90)));

    let decoded = without(&job, &["submittedAt", "processingAt", "promotedAt"]);

    assert_eq!(decoded.submitted_at, None);
    assert_eq!(decoded.processing_at, None);
    assert_eq!(decoded.promoted_at, None);
    assert_eq!(decoded.queue_wait(), None);
    assert_eq!(decoded.processing_time(), None);
}
