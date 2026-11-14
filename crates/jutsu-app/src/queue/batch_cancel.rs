//! Cancel All on a Queue batch header (`BatchCancel` in `BatchActions.swift`, `BatchHeader.swift`).

use jutsu_core::{Job, JobState, PauseReason};

use super::live::JobBatch;
use super::service::QueueConfirmation;
use crate::connection::UserMessage;

/// The batch's jobs that canceling still saves. Each goes through the engine's cancel, like the row's own Cancel.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchCancel {
    /// What Cancel All cancels, in queue order. A job Higgsfield is generating can't be canceled, and one that is
    /// downloading (or paused for disk space) is already charged, so stopping it saves nothing; both are left alone.
    pub jobs: Vec<Job>,
    /// How many of `jobs` were already sent to Higgsfield.
    pub submitted: usize,
}

impl BatchCancel {
    pub fn new(batch: &JobBatch) -> Self {
        let jobs: Vec<Job> = batch
            .jobs
            .iter()
            .filter(|job| {
                job.state.is_cancellable()
                    && job.state != JobState::Downloading
                    && job.state != JobState::Paused(PauseReason::NotEnoughDiskSpace)
            })
            .cloned()
            .collect();
        let submitted = jobs
            .iter()
            .filter(|job| job.remote.is_some() || job.submit_attempt.is_some())
            .count();
        Self { jobs, submitted }
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Asked first once any job is with Higgsfield; `None` while none was sent, when canceling only drops them here.
    pub fn confirmation(&self) -> Option<QueueConfirmation> {
        if self.submitted == 0 {
            return None;
        }
        let count = self.jobs.len();
        let (jobs, title_jobs) = if count == 1 {
            ("job", "Job")
        } else {
            ("jobs", "Jobs")
        };
        let verb = if self.submitted == 1 { "is" } else { "are" };
        Some(QueueConfirmation {
            title: format!("Cancel {count} {jobs}?"),
            message: format!(
                "{} of them {verb} already with Higgsfield, which is asked to drop them. A job it already started \
                 can't be canceled and is charged.",
                self.submitted
            ),
            confirm_title: format!("Cancel {count} {title_jobs}"),
        })
    }

    /// The alert after Cancel All: `None` when every job was canceled; the engine's own alert when the only job
    /// refused; else "2 of 4 jobs weren't canceled" with the first reason.
    pub fn outcome(refusals: &[UserMessage], total: usize) -> Option<UserMessage> {
        let first = refusals.first()?;
        if total == 1 {
            return Some(first.clone());
        }
        Some(UserMessage {
            title: format!("{} of {total} jobs weren't canceled", refusals.len()),
            message: format!("{}. {}", first.title, first.message),
            action: first.action,
        })
    }
}

#[cfg(test)]
#[path = "batch_cancel_tests.rs"]
mod tests;
