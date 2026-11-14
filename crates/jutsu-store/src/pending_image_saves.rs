//! `pending-images.json`: the recovery journal of paid images awaiting download.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jutsu_core::PendingImage;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::StoreError;
use crate::files::write_atomically;

const FILE_NAME: &str = "pending-images.json";

/// Recovery journal for paid images awaiting download; completed library entries live only in the catalog.
/// Calls run one at a time, as on the Swift actor.
pub struct PendingImageSaves {
    path: PathBuf,
    turn: Mutex<()>,
}

impl PendingImageSaves {
    /// The journal in `folder` (`FileLayout::root`).
    pub fn new(folder: &Path) -> Self {
        Self {
            path: folder.join(FILE_NAME),
            turn: Mutex::new(()),
        }
    }

    /// The images still to save, oldest first. A file that can't be read fails, so it is never written over.
    pub async fn pending(&self) -> Result<Vec<PendingImage>, StoreError> {
        let _turn = self.turn.lock().await;
        self.read().await
    }

    /// Adds the images of `new` that aren't in the journal yet.
    pub async fn add_pending(&self, new: Vec<PendingImage>) -> Result<(), StoreError> {
        let _turn = self.turn.lock().await;
        let mut images = self.read().await?;
        for image in new {
            if !images.iter().any(|kept| kept.id == image.id) {
                images.push(image);
            }
        }
        self.write(&images).await
    }

    /// Removes the images of `ids`.
    pub async fn remove_pending(&self, ids: &HashSet<Uuid>) -> Result<(), StoreError> {
        let _turn = self.turn.lock().await;
        let images = self.read().await?;
        if !images.iter().any(|image| ids.contains(&image.id)) {
            return Ok(());
        }
        let kept: Vec<PendingImage> = images
            .into_iter()
            .filter(|image| !ids.contains(&image.id))
            .collect();
        self.write(&kept).await
    }

    async fn read(&self) -> Result<Vec<PendingImage>, StoreError> {
        let data = match tokio::fs::read(&self.path).await {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        serde_json::from_slice(&data).map_err(|source| StoreError::File {
            file: FILE_NAME.to_owned(),
            source,
        })
    }

    async fn write(&self, images: &[PendingImage]) -> Result<(), StoreError> {
        // Through a `Value`, whose maps keep their keys sorted, as Swift's `.sortedKeys` writes them.
        let value = serde_json::to_value(images).map_err(StoreError::encode(FILE_NAME))?;
        let data = serde_json::to_vec_pretty(&value).map_err(StoreError::encode(FILE_NAME))?;
        Ok(write_atomically(&self.path, &data).await?)
    }
}
