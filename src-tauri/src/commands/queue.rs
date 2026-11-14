//! The Queue screen: its snapshot, the job commands, Pause all / Resume all, the "Didn't finish" section, Duplicate,
//! Show in File Manager, and the progress channel.

use std::sync::{Mutex, PoisonError};

use jutsu_app::queue::{DuplicateOutcome, JobCommand, JobProgress, QueueConfirmation, QueueSnapshot};
use tauri::AppHandle;
use tauri::async_runtime::JoinHandle;
use tauri::ipc::Channel;
use tauri_plugin_opener::OpenerExt;
use time::OffsetDateTime;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use super::Environment;
use crate::error::AppError;

/// The task feeding the window's progress channel; a new channel replaces it.
static PROGRESS: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

fn job_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError::NotFound("This job is no longer in the queue.".to_owned()))
}

/// Everything the Queue shows now. `QueueChanged` says when to ask again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_snapshot(environment: Environment<'_>) -> Result<QueueSnapshot, AppError> {
    Ok(environment.queue.snapshot(OffsetDateTime::now_utc()).await)
}

/// Runs a row's command on job `id`. A refusal shows as the connection alert.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_job_command(
    environment: Environment<'_>,
    id: String,
    command: JobCommand,
) -> Result<(), AppError> {
    environment.queue.perform(job_id(&id)?, command).await;
    Ok(())
}

/// Pauses every job not submitted yet.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_pause_all(environment: Environment<'_>) -> Result<(), AppError> {
    environment.queue.pause_all().await;
    Ok(())
}

/// Resumes the jobs Pause all paused.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_resume_all(environment: Environment<'_>) -> Result<(), AppError> {
    environment.queue.resume_all().await;
    Ok(())
}

/// Hides job `id` from "Didn't finish"; its record and cost stay.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_remove_problem(environment: Environment<'_>, id: String) -> Result<(), AppError> {
    environment
        .queue
        .remove_problem(job_id(&id)?)
        .await
        .map_err(AppError::from)
}

/// "Clear all": hides every job that didn't finish until now.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_clear_problems(environment: Environment<'_>) -> Result<(), AppError> {
    environment
        .queue
        .clear_problems(OffsetDateTime::now_utc())
        .map_err(AppError::from)
}

/// Queues job `id`'s request again. Over today's cap it returns the question; ask, then call again with
/// `over_cap_approved`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_duplicate(
    environment: Environment<'_>,
    id: String,
    over_cap_approved: bool,
) -> Result<DuplicateOutcome, AppError> {
    Ok(environment
        .queue
        .duplicate(job_id(&id)?, over_cap_approved, OffsetDateTime::now_utc())
        .await)
}

/// Shows job `id`'s result in the file manager.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_show_in_folder(
    app: AppHandle,
    environment: Environment<'_>,
    id: String,
) -> Result<(), AppError> {
    let path = environment
        .queue
        .output_path(job_id(&id)?)
        .ok_or_else(|| AppError::NotFound("This job has no result file.".to_owned()))?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|error| AppError::Internal(error.to_string()))
}

/// Sends each job's step progress on `on_progress` until the window drops it or opens another channel.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_progress(
    environment: Environment<'_>,
    on_progress: Channel<JobProgress>,
) -> Result<(), AppError> {
    let mut updates = environment.queue.subscribe_progress();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            match updates.recv().await {
                Ok(progress) => {
                    if on_progress.send(progress).is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    });
    let previous = PROGRESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(task);
    if let Some(previous) = previous {
        previous.abort();
    }
    Ok(())
}

/// Cancel All on batch `id`. Once a job is with Higgsfield it returns the question; ask, then call again
/// `approved`. Refusals show as the connection alert.
#[tauri::command]
#[specta::specta]
pub(crate) async fn queue_cancel_all(
    environment: Environment<'_>,
    id: String,
    approved: bool,
) -> Result<Option<QueueConfirmation>, AppError> {
    let batch = Uuid::parse_str(&id)
        .map_err(|_| AppError::NotFound("This batch is no longer in the queue.".to_owned()))?;
    Ok(environment.queue.cancel_all(batch, approved).await)
}
