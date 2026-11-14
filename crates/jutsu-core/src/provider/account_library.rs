//! What an account keeps besides its generations: likes, credit movements and uploads.

use async_trait::async_trait;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use url::Url;

use crate::models::PixelSize;
use crate::ports::PortError;
use crate::provider::HistoryCursor;
use crate::support::Seconds;
use crate::support::string_enum;

/// A provider whose account likes its generations; the library's star follows the like.
#[async_trait]
pub trait GenerationLikes: Send + Sync {
    /// Sets the like of the generation `RemoteJob::request_id` names; setting the state it has is harmless.
    async fn set_liked(&self, request_id: String, is_liked: bool) -> Result<(), PortError>;
}

/// A provider that reports the account's credit movements.
#[async_trait]
pub trait CreditHistory: Send + Sync {
    /// One page, newest first; `None` for the first page.
    async fn credit_history(&self, after: Option<HistoryCursor>) -> Result<CreditPage, PortError>;
}

/// A provider whose account keeps the files uploaded to it, to start new jobs from.
#[async_trait]
pub trait AccountUploads: Send + Sync {
    /// One page of `kind`, newest first; `None` for the first page.
    async fn uploads(&self, kind: UploadKind, after: Option<HistoryCursor>) -> Result<UploadPage, PortError>;
}

string_enum! {
    /// What a credit movement was.
    pub enum CreditEntryKind {
        /// Spent.
        Spend = "spend",
        /// Refunded.
        Refund = "refund",
        /// Granted.
        Grant = "grant",
        /// Deducted.
        Deduct = "deduct",
        /// Anything else.
        Other = "other",
    }
}

/// One movement of the account's credits. The provider names no job, only what the credits were for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CreditEntry {
    /// When.
    pub date: OffsetDateTime,
    /// Negative when spent, positive when refunded or granted.
    pub credits: Decimal,
    /// What it was.
    pub kind: CreditEntryKind,
    /// "Higgsfield Genjutsu - Object Swap".
    pub title: String,
}

/// One page of credit movements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CreditPage {
    /// The movements.
    pub items: Vec<CreditEntry>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}

string_enum! {
    /// What an upload is.
    pub enum UploadKind {
        /// An image.
        Image = "image",
        /// A video.
        Video = "video",
    }
}

/// A file the account uploaded, downloadable while the provider keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadedFile {
    /// The provider's media ID.
    pub id: String,
    /// Image or video.
    pub kind: UploadKind,
    /// The file.
    pub url: Url,
    /// Videos only.
    pub thumbnail_url: Option<Url>,
    /// Its frame.
    pub pixel_size: Option<PixelSize>,
    /// Videos only, in seconds.
    pub duration: Option<Seconds>,
    /// When it was uploaded.
    pub created_at: OffsetDateTime,
}

/// One page of uploads.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadPage {
    /// The uploads.
    pub items: Vec<UploadedFile>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}
