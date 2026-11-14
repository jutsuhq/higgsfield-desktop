//! `AssetRecord`: uploaded inputs, remembered by the SHA-256 of the prepared file so they aren't uploaded twice.

use async_trait::async_trait;
use jutsu_core::{PortError, RemoteAsset, UploadCache};
use sqlx::Row;
use time::OffsetDateTime;
use url::Url;

use crate::error::StoreError;
use crate::store::{Store, nanos, text};

impl Store {
    /// The asset uploaded for `content_hash` at or after `uploaded_after`, if any: reused only within the upload's
    /// lifetime, which the caller passes as the earliest date.
    pub async fn asset(
        &self,
        content_hash: &str,
        uploaded_after: OffsetDateTime,
    ) -> Result<Option<RemoteAsset>, StoreError> {
        let row = sqlx::query(
            "SELECT remote_url, remote_id FROM assets WHERE content_hash = ? AND uploaded_at >= ?",
        )
        .bind(content_hash)
        .bind(nanos(uploaded_after))
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else { return Ok(None) };
        let url = Url::parse(&text(&row, "remote_url")?)
            .map_err(|error| StoreError::damaged("assets", content_hash, error))?;
        Ok(Some(RemoteAsset {
            url,
            id: row.try_get("remote_id")?,
        }))
    }

    /// Remembers `asset` for `content_hash`, replacing the one stored for it.
    pub async fn store_asset(
        &self,
        asset: &RemoteAsset,
        content_hash: &str,
        uploaded_at: OffsetDateTime,
    ) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query(
            "INSERT INTO assets (content_hash, remote_url, remote_id, uploaded_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (content_hash) DO UPDATE SET remote_url = excluded.remote_url, \
             remote_id = excluded.remote_id, uploaded_at = excluded.uploaded_at",
        )
        .bind(content_hash)
        .bind(asset.url.as_str())
        .bind(&asset.id)
        .bind(nanos(uploaded_at))
        .execute(&mut *write.transaction)
        .await?;
        write.commit(&[]).await
    }

    /// Forgets the assets uploaded before `uploaded_before`.
    pub async fn remove_assets(&self, uploaded_before: OffsetDateTime) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query("DELETE FROM assets WHERE uploaded_at < ?")
            .bind(nanos(uploaded_before))
            .execute(&mut *write.transaction)
            .await?;
        write.commit(&[]).await
    }
}

#[async_trait]
impl UploadCache for Store {
    async fn asset(
        &self,
        content_hash: &str,
        uploaded_after: OffsetDateTime,
    ) -> Result<Option<RemoteAsset>, PortError> {
        Ok(Self::asset(self, content_hash, uploaded_after).await?)
    }

    async fn store(
        &self,
        asset: &RemoteAsset,
        content_hash: &str,
        uploaded_at: OffsetDateTime,
    ) -> Result<(), PortError> {
        Ok(self.store_asset(asset, content_hash, uploaded_at).await?)
    }

    async fn remove_assets(&self, uploaded_before: OffsetDateTime) -> Result<(), PortError> {
        Ok(Self::remove_assets(self, uploaded_before).await?)
    }
}
