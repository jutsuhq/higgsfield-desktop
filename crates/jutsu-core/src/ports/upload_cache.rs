//! Uploads remembered by content.

use async_trait::async_trait;
use time::OffsetDateTime;

use crate::ports::PortError;
use crate::provider::RemoteAsset;

/// Remembers uploads by content hash so the same input isn't uploaded twice.
#[async_trait]
pub trait UploadCache: Send + Sync {
    /// The asset uploaded for `content_hash` at or after `date`, if any.
    async fn asset(
        &self,
        content_hash: &str,
        uploaded_after: OffsetDateTime,
    ) -> Result<Option<RemoteAsset>, PortError>;
    /// Remembers `asset` for `content_hash`.
    async fn store(
        &self,
        asset: &RemoteAsset,
        content_hash: &str,
        uploaded_at: OffsetDateTime,
    ) -> Result<(), PortError>;
    /// Forgets the assets uploaded before `date`.
    async fn remove_assets(&self, uploaded_before: OffsetDateTime) -> Result<(), PortError>;
}
