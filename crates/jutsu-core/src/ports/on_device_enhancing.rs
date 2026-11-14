//! Enhance on this computer.

use async_trait::async_trait;
use url::Url;

use crate::ports::{PortError, Progress};
use crate::rules::{OnDeviceAction, OnDeviceEngine, OnDeviceOutput, OnDeviceProgress};

/// Enhance on this computer (`OnDeviceAction`): free, no account, no upload. On macOS, MediaKit runs it on
/// VideoToolbox; another platform reports no engine.
#[async_trait]
pub trait OnDeviceEnhancing: Send + Sync {
    /// Whether this computer has the engine. Asks the system only whether it is supported, never about its model,
    /// which asking would start downloading.
    fn supports(&self, engine: OnDeviceEngine) -> bool;
    /// Whether the engine's model is known to be on this computer. Asking may start the system's download of it, so
    /// it is asked only once a run on the engine ended, after the person agreed to the download
    /// (`OnDeviceEngine::downloads_model`).
    fn model_is_ready(&self, _engine: OnDeviceEngine) -> bool {
        false
    }
    /// Makes `action`'s result from the file at `source`, written to `output`. Canceling or failing leaves nothing
    /// at `output`.
    async fn run(
        &self,
        action: &OnDeviceAction,
        source: &Url,
        output: &Url,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<OnDeviceOutput, PortError>;
}
