//! Downloading a model's files.

use std::path::Path;

use async_trait::async_trait;

use crate::models::ModelManifest;
use crate::ports::{PortError, Progress};

/// Downloads a model's files (`ModelManifest`) into a folder of the app's own, checks them, and removes them.
#[async_trait]
pub trait SpeechModelDownloading: Send + Sync {
    /// Whether every file is in `folder` at its size; reads no content.
    fn is_complete(&self, manifest: &ModelManifest, folder: &Path) -> bool;
    /// Downloads the files that aren't complete yet, each resuming where an earlier download stopped, and checks
    /// each against its hash; a file that doesn't match is deleted and the download fails. `progress` is 0…1 of
    /// the whole model.
    async fn download(
        &self,
        manifest: &ModelManifest,
        folder: &Path,
        progress: Progress<f64>,
    ) -> Result<(), PortError>;
    /// What `folder` takes on disk, in bytes.
    fn size(&self, folder: &Path) -> i64;
    /// Removes `folder`.
    fn remove(&self, folder: &Path) -> Result<(), PortError>;
}
