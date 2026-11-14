//! Ready-made source clips.

use async_trait::async_trait;
use url::Url;

use crate::models::Mode;
use crate::ports::PortError;
use crate::provider::HistoryCursor;
use crate::support::string_enum;

/// A provider that offers ready-made source clips to run Motion or Swap on (Higgsfield's Motion Library), and the
/// videos the account uploaded before.
#[async_trait]
pub trait MotionLibrary: Send + Sync {
    /// One page of `shelf`, in the provider's order; `None` for the first page.
    async fn motion_presets(
        &self,
        shelf: MotionShelf,
        after: Option<HistoryCursor>,
    ) -> Result<MotionPresetPage, PortError>;
}

string_enum! {
    /// The tabs of the library.
    pub enum MotionShelf {
        /// Picked by Higgsfield.
        Featured = "featured",
        /// The community's most used.
        Trending = "trending",
        /// The community's newest.
        New = "new",
        /// Videos the account uploaded, as sources of past jobs or on higgsfield.ai.
        Uploads = "uploads",
    }
}

/// A source clip from the library, with the example made from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MotionPreset {
    /// The provider's ID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The clip a job starts from.
    pub video_url: Url,
    /// A small picture.
    pub thumbnail_url: Option<Url>,
    /// The example result, a short video.
    pub preview_url: Option<Url>,
    /// What the example did with the clip; `None` when the provider didn't say. Any mode can use it.
    pub example_mode: Option<Mode>,
}

/// One page of the library.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MotionPresetPage {
    /// The clips.
    pub items: Vec<MotionPreset>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}
