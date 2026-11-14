//! Uploaded inputs of one job.

use serde::{Deserialize, Serialize};

use crate::provider::RemoteAsset;

/// Uploaded inputs of one job, in the order of `JobSpec::references`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoteInputs {
    /// `None` for a clip made without a source video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<RemoteAsset>,
    /// The reference images.
    pub images: Vec<RemoteAsset>,
    /// The uploaded speech (`JobSpec::speech`); `None` without one, and in jobs stored before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<RemoteAsset>,
}
