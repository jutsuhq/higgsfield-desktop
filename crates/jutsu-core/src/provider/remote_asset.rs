//! An uploaded file.

use serde::{Deserialize, Serialize};
use url::Url;

/// An uploaded file, reachable by the provider at `url`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoteAsset {
    /// Where the provider reaches it.
    pub url: Url,
    /// The provider's own ID for the file, when requests name it by ID rather than URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl RemoteAsset {
    /// The asset at `url`, without an ID.
    pub fn new(url: Url) -> Self {
        Self { url, id: None }
    }
}
