//! Durable job storage.

use std::collections::HashSet;

use async_trait::async_trait;
use uuid::Uuid;

use crate::models::Job;
use crate::ports::PortError;

/// Durable job storage. The engine saves every state change before acting on it, so a crash at any point resumes
/// from the last saved state.
#[async_trait]
pub trait JobRepository: Send + Sync {
    /// Saves `job`, replacing the stored one of the same ID.
    async fn save(&self, job: &Job) -> Result<(), PortError>;
    /// The stored job of `id`.
    async fn job(&self, id: Uuid) -> Result<Option<Job>, PortError>;
    /// Jobs that are not terminal, oldest first; resumed on launch.
    async fn active_jobs(&self) -> Result<Vec<Job>, PortError>;
    /// IDs of every stored job that is not terminal, including records `active_jobs` skips because they don't
    /// decode, so their scratch files are kept for a build that can read them.
    async fn active_job_ids(&self) -> Result<HashSet<Uuid>, PortError>;
    /// Removes the job of `id`.
    async fn delete(&self, id: Uuid) -> Result<(), PortError>;
}
