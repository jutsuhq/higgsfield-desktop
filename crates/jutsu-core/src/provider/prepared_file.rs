//! A local file ready to upload.

use serde::{Deserialize, Serialize};
use url::Url;

/// A local file ready to upload: trimmed and transcoded video, or a converted image.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct PreparedFile {
    /// The file.
    #[serde(rename = "fileURL")]
    pub file_url: Url,
    /// MIME type, e.g. `video/mp4`.
    pub content_type: String,
    /// Its size.
    pub byte_count: i64,
    /// Hex SHA-256 of the contents; key of the upload cache.
    pub content_hash: String,
}
