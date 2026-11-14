//! `SwiftDataStoreTests`: job records, the job queries and the upload cache.

use std::collections::HashSet;

use jutsu_core::{
    CostUnit, FailureReason, JobRepository, JobState, Mode, PauseReason, RemoteAsset, RemoteJob, UploadCache,
};
use jutsu_store::{JobRecord, Store, StoreError};
use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime, UtcOffset};
use url::Url;
use uuid::Uuid;

use crate::common::{at, file_url, job, make_job, memory_store};

async fn records(store: &Store) -> Vec<JobRecord> {
    sqlx::query("SELECT * FROM jobs")
        .fetch_all(store.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| JobRecord::from_row(row).unwrap())
        .collect()
}

/// Rewrites a stored job as a newer build might: with a mode this build doesn't know.
async fn write_unknown_mode(store: &Store, id: Uuid) {
    let json: String = sqlx::query_scalar("SELECT job_data FROM jobs WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(json.contains("\"motionTransfer\""));
    let rewritten = json.replace("\"motionTransfer\"", "\"fromTheFuture\"");
    sqlx::query("UPDATE jobs SET job_data = ? WHERE id = ?")
        .bind(rewritten)
        .bind(id.to_string())
        .execute(store.pool())
        .await
        .unwrap();
    let record = records(store)
        .await
        .into_iter()
        .find(|record| record.id == id)
        .unwrap();
    assert!(record.job().is_err());
}

fn failed() -> JobState {
    JobState::Failed {
        reason: FailureReason::Message("x".to_owned()),
    }
}

#[tokio::test]
async fn saves_and_fetches_job() {
    let store = memory_store().await;
    let mut job = job(JobState::Preparing);
    let mut remote = RemoteJob::new("r1");
    remote.correlation_id = Some("c1".to_owned());
    job.remote = Some(remote);
    store.save_job(&job).await.unwrap();

    assert_eq!(store.job(job.id).await.unwrap(), Some(job));
    assert_eq!(store.job(Uuid::new_v4()).await.unwrap(), None);
}

#[tokio::test]
async fn save_upserts_by_id() {
    let store = memory_store().await;
    let mut job = job(JobState::Preparing);
    store.save_job(&job).await.unwrap();
    job.state = JobState::Done;
    job.progress = 1.0;
    job.finished_at = Some(at(1_000));
    job.output_file = Some(file_url("/tmp/out.mp4"));
    store.save_job(&job).await.unwrap();

    assert_eq!(store.job(job.id).await.unwrap(), Some(job));
    let records = records(&store).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state_kind, "done");
    assert!(records[0].is_terminal);
    let output = file_url("/tmp/out.mp4").to_file_path().unwrap();
    assert_eq!(records[0].output_path.as_deref(), output.to_str());
    assert_eq!(records[0].finished_at, Some(at(1_000)));
}

#[tokio::test]
async fn star_survives_the_engine_saving_the_job_again() {
    let store = memory_store().await;
    let mut job = job(JobState::Done);
    job.output_file = Some(file_url("/tmp/result.mp4"));
    store.save_job(&job).await.unwrap();
    sqlx::query("UPDATE materials SET is_starred = 1 WHERE id = ?")
        .bind(job.id.to_string())
        .execute(store.pool())
        .await
        .unwrap();
    store.save_job(&job).await.unwrap();
    assert!(store.material(job.id).await.unwrap().unwrap().is_starred);
}

#[tokio::test]
async fn record_keeps_query_columns() {
    let store = memory_store().await;
    let job = make_job(
        Mode::Restyle,
        OffsetDateTime::now_utc(),
        JobState::Failed {
            reason: FailureReason::Message("bad input".to_owned()),
        },
    );
    store.save_job(&job).await.unwrap();

    let record = records(&store).await.remove(0);
    assert_eq!(record.id, job.id);
    assert_eq!(record.state_kind, "failed");
    assert!(record.is_terminal);
    assert!(!record.is_draft);
    assert_eq!(record.mode, "restyle");
    assert_eq!(record.resolution, "720p");
    assert_eq!(record.provider_kind, "apiKey");
    assert_eq!(record.prompt, "a cat");
    assert_eq!(record.output_path, None);
    assert_eq!(record.job().unwrap(), job);
}

#[test]
fn update_rejects_another_job() {
    let mut record = JobRecord::new(&job(JobState::Preparing), &[]).unwrap();
    assert!(matches!(
        record.update(&job(JobState::Preparing)),
        Err(StoreError::JobIdMismatch)
    ));
}

#[tokio::test]
async fn active_jobs_are_non_terminal_oldest_first() {
    let store = memory_store().await;
    let base = at(1_000_000);
    let newer = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(60),
        JobState::Queued,
    );
    let older = make_job(
        Mode::MotionTransfer,
        base,
        JobState::Paused(PauseReason::InsufficientFunds),
    );
    let done = make_job(Mode::MotionTransfer, base - Duration::seconds(60), JobState::Done);
    let canceled = make_job(
        Mode::MotionTransfer,
        base - Duration::seconds(30),
        JobState::Canceled,
    );
    for job in [&newer, &done, &older, &canceled] {
        store.save_job(job).await.unwrap();
    }

    let ids: Vec<Uuid> = store
        .active_jobs()
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [older.id, newer.id]);
}

#[tokio::test]
async fn active_jobs_skip_undecodable_records_and_keep_them() {
    let store = memory_store().await;
    let base = at(1_000_000);
    let older = make_job(Mode::MotionTransfer, base, JobState::Queued);
    let unreadable = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(30),
        JobState::Processing,
    );
    let newer = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(60),
        JobState::Processing,
    );
    for job in [&older, &unreadable, &newer] {
        store.save_job(job).await.unwrap();
    }
    write_unknown_mode(&store, unreadable.id).await;

    let ids: Vec<Uuid> = store
        .active_jobs()
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [older.id, newer.id]);
    let all = HashSet::from([older.id, unreadable.id, newer.id]);
    assert_eq!(JobRepository::active_job_ids(&store).await.unwrap(), all);
    let kept: HashSet<Uuid> = records(&store).await.iter().map(|record| record.id).collect();
    assert_eq!(kept, all);
}

#[tokio::test]
async fn spending_rejects_an_incomplete_history_while_the_history_still_lists_readable_jobs() {
    let store = memory_store().await;
    let start_of_day = at(1_000_000);
    let mut done = make_job(Mode::MotionTransfer, start_of_day, JobState::Done);
    done.finished_at = Some(start_of_day + Duration::seconds(60));
    let mut unreadable = make_job(
        Mode::MotionTransfer,
        start_of_day + Duration::seconds(30),
        JobState::Done,
    );
    unreadable.finished_at = Some(start_of_day + Duration::seconds(90));
    for job in [&done, &unreadable] {
        store.save_job(job).await.unwrap();
    }
    write_unknown_mode(&store, unreadable.id).await;

    assert!(
        store
            .committed_spend(start_of_day, CostUnit::Credits, None, UtcOffset::UTC)
            .await
            .is_err()
    );
    let ids: Vec<Uuid> = store
        .done_jobs(5)
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [done.id]);
}

#[tokio::test]
async fn unfinished_or_finished_since_skips_older_results() {
    let store = memory_store().await;
    let start_of_day = at(1_000_000);
    let running = job(JobState::Processing);
    let mut today = job(JobState::Done);
    today.finished_at = Some(start_of_day + Duration::seconds(60));
    let mut yesterday = job(JobState::Done);
    yesterday.finished_at = Some(start_of_day - Duration::seconds(60));
    for job in [&running, &today, &yesterday] {
        store.save_job(job).await.unwrap();
    }

    let ids: HashSet<Uuid> = store
        .jobs_unfinished_or_finished(start_of_day)
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, HashSet::from([running.id, today.id]));
}

#[tokio::test]
async fn committed_spend_counts_today_and_the_queue_in_one_unit() {
    let store = memory_store().await;
    let now = at(86_400 * 100 + 3600);
    let running = job(JobState::Processing);
    let other = job(JobState::Queued);
    let mut today = job(JobState::Done);
    today.finished_at = Some(now - Duration::seconds(60));
    let mut yesterday = job(JobState::Done);
    yesterday.finished_at = Some(now - Duration::seconds(86_400));
    for job in [&running, &other, &today, &yesterday] {
        store.save_job(job).await.unwrap();
    }

    let utc = UtcOffset::UTC;
    let all = store
        .committed_spend(now, CostUnit::Credits, None, utc)
        .await
        .unwrap();
    let without_other = store
        .committed_spend(now, CostUnit::Credits, Some(other.id), utc)
        .await
        .unwrap();
    let usd = store
        .committed_spend(now, CostUnit::Usd, None, utc)
        .await
        .unwrap();
    assert_eq!(all, Decimal::from(36));
    assert_eq!(without_other, Decimal::from(24));
    assert_eq!(usd, Decimal::ZERO);
}

#[tokio::test]
async fn finished_newest_first_lists_terminal_jobs() {
    let store = memory_store().await;
    let base = at(1_000_000);
    let first = make_job(Mode::MotionTransfer, base, JobState::Done);
    let second = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(60),
        JobState::Rejected,
    );
    let running = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(120),
        JobState::Processing,
    );
    for job in [&first, &running, &second] {
        store.save_job(job).await.unwrap();
    }

    let ids: Vec<Uuid> = store
        .finished_jobs()
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [second.id, first.id]);
}

#[tokio::test]
async fn problems_are_failed_rejected_and_canceled_newest_finished_first() {
    let store = memory_store().await;
    let base = at(1_000_000);
    let finished = |state: JobState, minutes: i64| {
        let mut job = make_job(Mode::MotionTransfer, base, state);
        job.finished_at = Some(base + Duration::minutes(minutes));
        job
    };
    let cleared = finished(failed(), 1);
    let failed = finished(failed(), 3);
    let canceled = finished(JobState::Canceled, 4);
    let rejected = finished(JobState::Rejected, 5);
    let done = finished(JobState::Done, 6);
    let running = make_job(Mode::MotionTransfer, base, JobState::Processing);
    for job in [&cleared, &done, &failed, &running, &rejected, &canceled] {
        store.save_job(job).await.unwrap();
    }

    let after = base + Duration::minutes(2);
    let all: Vec<Uuid> = store
        .problem_jobs(Some(after), 10)
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(all, [rejected.id, canceled.id, failed.id]);
    let limited: Vec<Uuid> = store
        .problem_jobs(None, 2)
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(limited, [rejected.id, canceled.id]);
}

#[tokio::test]
async fn done_jobs_are_newest_first_and_limited() {
    let store = memory_store().await;
    let base = at(1_000_000);
    let older = make_job(Mode::MotionTransfer, base, JobState::Done);
    let newer = make_job(Mode::MotionTransfer, base + Duration::seconds(60), JobState::Done);
    let newest = make_job(
        Mode::MotionTransfer,
        base + Duration::seconds(120),
        JobState::Done,
    );
    let failed = make_job(Mode::MotionTransfer, base + Duration::seconds(180), failed());
    for job in [&older, &failed, &newest, &newer] {
        store.save_job(job).await.unwrap();
    }

    let ids: Vec<Uuid> = store
        .done_jobs(2)
        .await
        .unwrap()
        .iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [newest.id, newer.id]);
}

#[tokio::test]
async fn deletes_job() {
    let store = memory_store().await;
    let job = job(JobState::Preparing);
    JobRepository::save(&store, &job).await.unwrap();
    JobRepository::delete(&store, job.id).await.unwrap();
    JobRepository::delete(&store, job.id).await.unwrap();

    assert_eq!(JobRepository::job(&store, job.id).await.unwrap(), None);
    assert!(JobRepository::active_jobs(&store).await.unwrap().is_empty());
}

fn remote_asset(url: &str, id: Option<&str>) -> RemoteAsset {
    RemoteAsset {
        url: Url::parse(url).unwrap(),
        id: id.map(str::to_owned),
    }
}

#[tokio::test]
async fn upload_cache_filters_by_date() {
    let store = memory_store().await;
    let uploaded_at = at(1_000_000);
    let asset = remote_asset("https://cdn.example.com/a.mp4", Some("media-1"));
    UploadCache::store(&store, &asset, "abc", uploaded_at)
        .await
        .unwrap();

    let cache: &dyn UploadCache = &store;
    assert_eq!(
        cache.asset("abc", uploaded_at).await.unwrap(),
        Some(asset.clone())
    );
    assert_eq!(
        cache
            .asset("abc", uploaded_at - Duration::seconds(1))
            .await
            .unwrap(),
        Some(asset)
    );
    assert_eq!(
        cache
            .asset("abc", uploaded_at + Duration::seconds(1))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        cache.asset("other", OffsetDateTime::UNIX_EPOCH).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn upload_cache_replaces_asset_for_same_hash() {
    let store = memory_store().await;
    let first = remote_asset("https://cdn.example.com/1.mp4", None);
    let second = remote_asset("https://cdn.example.com/2.mp4", Some("media-2"));
    store.store_asset(&first, "abc", at(1)).await.unwrap();
    store.store_asset(&second, "abc", at(2)).await.unwrap();

    assert_eq!(
        store.asset("abc", OffsetDateTime::UNIX_EPOCH).await.unwrap(),
        Some(second)
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn removing_old_assets_keeps_the_ones_uploaded_since() {
    let store = memory_store().await;
    let cutoff = at(1_000_000);
    let old = remote_asset("https://cdn.example.com/old.mp4", None);
    let fresh = remote_asset("https://cdn.example.com/fresh.mp4", None);
    store
        .store_asset(&old, "old", cutoff - Duration::seconds(1))
        .await
        .unwrap();
    store.store_asset(&fresh, "fresh", cutoff).await.unwrap();

    store.remove_assets(cutoff).await.unwrap();

    assert_eq!(
        store.asset("old", OffsetDateTime::UNIX_EPOCH).await.unwrap(),
        None
    );
    assert_eq!(
        store.asset("fresh", OffsetDateTime::UNIX_EPOCH).await.unwrap(),
        Some(fresh)
    );
}
