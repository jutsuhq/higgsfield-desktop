//! Restyle presets.

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

/// A restyle preset from the provider's catalog.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct StylePreset {
    /// Its identity.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// A still image.
    #[serde(rename = "previewURL")]
    pub preview_url: Url,
    /// A short example video, played on hover; `None` when the provider offers none.
    #[serde(default, rename = "previewVideoURL", skip_serializing_if = "Option::is_none")]
    pub preview_video_url: Option<Url>,
}
