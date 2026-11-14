//! An account's reusable reference elements.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::ports::PortError;
use crate::provider::{HistoryCursor, RemoteAsset};
use crate::support::string_enum;

/// A provider whose account keeps reusable references: named elements (characters, environments, props), each with
/// images that can be used as reference images.
#[async_trait]
pub trait AccountElements: Send + Sync {
    /// `GenerationProvider::owner` of the account the elements belong to.
    fn owner(&self) -> Option<String>;
    /// One page of the elements, in the provider's order; `None` for the first page.
    async fn elements(&self, after: Option<HistoryCursor>) -> Result<AccountElementPage, PortError>;
    /// `element` with all its images, for one whose page named none.
    async fn element_images(&self, element: AccountElement) -> Result<AccountElement, PortError>;
    /// Publishes `images`, uploads of this account (`GenerationProvider::upload`), as a new element named `name`.
    /// Free. Not idempotent: sent again, it makes a second element.
    async fn create_element(
        &self,
        name: String,
        category: ElementCategory,
        images: Vec<RemoteAsset>,
    ) -> Result<AccountElement, PortError>;
}

string_enum! {
    /// What an element shows.
    pub enum ElementCategory {
        /// A character.
        Character = "character",
        /// An environment.
        Environment = "environment",
        /// A prop.
        Prop = "prop",
    }
}

/// A reference element of the account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountElement {
    /// The provider's ID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// `None` when the provider didn't say.
    pub category: Option<ElementCategory>,
    /// A small picture of it.
    pub thumbnail_url: Option<Url>,
    /// Its images.
    pub images: Vec<ElementImage>,
    /// The provider's state of it, such as `completed` once it can be used or `processing` while it is checked;
    /// `None` when the provider didn't say. Lists hold only usable ones.
    pub status: Option<String>,
}

/// One image of an element: its file on the provider and, when the provider named it, the media it already is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ElementImage {
    /// The file.
    pub url: Url,
    /// Sent instead of an upload of the downloaded file; `None` when the provider named no media ID.
    pub media: Option<ServerMedia>,
}

/// One page of elements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountElementPage {
    /// The elements.
    pub items: Vec<AccountElement>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}

/// A file the provider already keeps for one account, named by its media ID (an upload's, or a finished image job's
/// such as a character sheet): a job of that account sends the ID instead of uploading the file again.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ServerMedia {
    /// The media ID.
    pub id: String,
    /// The file.
    pub url: Url,
    /// `GenerationProvider::owner` of the account that listed it; another account's job uploads the file instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}
