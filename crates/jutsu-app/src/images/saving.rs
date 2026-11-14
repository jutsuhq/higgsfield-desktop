//! Saving each generated image to the output folder's `Images` folder and the Library as it arrives
//! (`ImageLibrary.swift`'s save, trash and pending saves).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jutsu_core::{MaterialGeneration, MediaLinkKind, OutputSubfolder, PendingImage, SavedImage};
use jutsu_engine::MediaLinkDownloader;
use jutsu_store::{FileLayout, PendingImageSaves, Store};
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::Notify;
use url::Url;
use uuid::Uuid;

use super::generator::GeneratedImage;
use crate::connection::UserMessage;
use crate::preferences::PreferencesStore;

/// Waits before trying a failed save again: most failures are a dropped connection.
const RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];

/// Called with each image saved and where.
pub type SavedHandler = Arc<dyn Fn(&GeneratedImage, PathBuf) + Send + Sync>;

/// Downloads generated images into the Library.
pub struct ImageSaver {
    store: Arc<Store>,
    layout: FileLayout,
    preferences: Arc<PreferencesStore>,
    offset: UtcOffset,
    downloader: MediaLinkDownloader,
    pending: PendingImageSaves,
    /// Saves in flight; a quit waits for them.
    saving: Mutex<usize>,
    finished: Notify,
    failure: Mutex<Option<UserMessage>>,
}

impl std::fmt::Debug for ImageSaver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ImageSaver").finish_non_exhaustive()
    }
}

impl ImageSaver {
    pub fn new(
        store: Arc<Store>,
        layout: FileLayout,
        preferences: Arc<PreferencesStore>,
        offset: UtcOffset,
    ) -> Arc<Self> {
        let pending = PendingImageSaves::new(&layout.root);
        Arc::new(Self {
            store,
            layout,
            preferences,
            offset,
            downloader: MediaLinkDownloader::default(),
            pending,
            saving: Mutex::new(0),
            finished: Notify::new(),
            failure: Mutex::new(None),
        })
    }

    /// `<output folder>/Images`.
    pub fn images_folder(&self) -> PathBuf {
        let folder = self
            .preferences
            .get()
            .output_folder
            .map_or_else(|| self.layout.default_output_folder.clone(), PathBuf::from);
        OutputSubfolder::Images.path(&folder)
    }

    /// The last save that failed, for an alert; taken once read.
    pub fn take_failure(&self) -> Option<UserMessage> {
        self.failure.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    pub fn failure(&self) -> Option<UserMessage> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Saves `images` one after another, reporting each one saved. They are noted first as pending, so one that
    /// can't be saved now is tried again on the next launch.
    pub fn save(
        self: &Arc<Self>,
        images: Vec<GeneratedImage>,
        created_at: HashMap<Uuid, OffsetDateTime>,
        saved: SavedHandler,
    ) {
        *self.saving.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let this = self.clone();
        tokio::spawn(async move {
            let now = OffsetDateTime::now_utc();
            let pending: Vec<PendingImage> = images
                .iter()
                .map(|image| PendingImage {
                    id: image.id,
                    remote_url: image.url.clone(),
                    prompt: image.prompt.clone(),
                    model: image.model.clone(),
                    aspect_ratio: image.aspect_ratio.clone(),
                    created_at: created_at.get(&image.id).copied().unwrap_or(now),
                    remote: image.remote.clone(),
                    parameters: image.parameters.clone(),
                    input_images: image.input_images.clone(),
                    variant_group_id: image.variant_group_id,
                    title: image.title.clone(),
                })
                .collect();
            if let Err(error) = this.pending.add_pending(pending.clone()).await {
                tracing::error!(%error, "can't note the images to save");
            }
            for (image, note) in images.iter().zip(&pending) {
                if let Some(file) = this.save_retrying(image, note.created_at).await {
                    saved(image, file);
                }
            }
            *this.saving.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
            this.finished.notify_waiters();
        });
    }

    /// Waits for the images being saved, so a quit doesn't cut off a download whose link expires.
    pub async fn finish_saving(&self) {
        loop {
            let finished = self.finished.notified();
            if *self.saving.lock().unwrap_or_else(PoisonError::into_inner) == 0 {
                return;
            }
            finished.await;
        }
    }

    async fn save_retrying(&self, image: &GeneratedImage, date: OffsetDateTime) -> Option<PathBuf> {
        let mut waits = RETRIES.iter();
        loop {
            if let Some(file) = self.save_one(image, date).await {
                let _ = self.pending.remove_pending(&HashSet::from([image.id])).await;
                return Some(file);
            }
            tokio::time::sleep(*waits.next()?).await;
        }
    }

    /// Downloads `image` into the `Images` folder, named after its prompt, and lists it in the Library; `None` when
    /// it couldn't be saved, and the failure says why.
    async fn save_one(&self, image: &GeneratedImage, date: OffsetDateTime) -> Option<PathBuf> {
        match self.download_and_save(image, date).await {
            Ok(file) => Some(file),
            Err(message) => {
                tracing::error!(%message, "can't save a generated image");
                *self.failure.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(UserMessage::new("Can't save this image", message, None));
                None
            }
        }
    }

    async fn download_and_save(
        &self,
        image: &GeneratedImage,
        date: OffsetDateTime,
    ) -> Result<PathBuf, String> {
        if let Some(remote) = &image.remote
            && let Ok(Some(known)) = self.store.material_by_remote(remote).await
            && let Some(file) = known
                .current
                .file_url
                .as_ref()
                .and_then(|url| url.to_file_path().ok())
            && file.exists()
        {
            return Ok(file);
        }
        let media = self
            .downloader
            .download(
                &image.url,
                &self.layout.generated_images_folder(),
                Some(MediaLinkKind::Image),
                &|_| {},
            )
            .await
            .map_err(|error| error.to_string())?;
        let folder = self.images_folder();
        let moved = move_into(&media.file, &folder, image, date, self.offset).await;
        if let Some(parent) = media.file.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
        let file = moved?;
        let url = Url::from_file_path(&file).map_err(|()| format!("Can't name “{}”.", file.display()))?;
        let saved = SavedImage {
            id: image.id,
            file_url: url,
            prompt: image.prompt.clone(),
            model: image.model.clone(),
            aspect_ratio: image.aspect_ratio.clone(),
            created_at: date,
            remote_url: Some(image.url.clone()),
            is_starred: false,
            title: image.title.clone(),
        };
        let generation = MaterialGeneration {
            prompt: image.prompt.clone(),
            model: image.model.clone(),
            parameters: image.parameters.clone(),
            title: image.title.clone(),
        };
        if let Err(error) = self
            .store
            .save_image(
                &saved,
                image.remote.clone(),
                Some(generation),
                &image.input_images,
                image.variant_group_id,
            )
            .await
        {
            // Not listed, so taken back: the next try saves it again instead of leaving a second copy.
            let _ = std::fs::remove_file(&file);
            return Err(error.to_string());
        }
        Ok(file)
    }

    /// Saves again the images a failed download or a quit left unsaved, while their links may still serve them.
    /// Run once, at launch.
    pub async fn resume_pending(self: &Arc<Self>, now: OffsetDateTime) {
        let pending = match self.pending.pending().await {
            Ok(pending) => pending,
            Err(error) => {
                tracing::error!(%error, "can't read the images left unsaved");
                return;
            }
        };
        let mut ended = HashSet::new();
        for image in &pending {
            let saved = matches!(
                self.store.material(image.id).await,
                Ok(Some(material)) if material.current.file_url.is_some()
            );
            if saved || now - image.created_at > PendingImage::LIFETIME {
                ended.insert(image.id);
            }
        }
        let _ = self.pending.remove_pending(&ended).await;
        let images: Vec<GeneratedImage> = pending
            .iter()
            .filter(|image| !ended.contains(&image.id))
            .map(|pending| GeneratedImage {
                id: pending.id,
                url: pending.remote_url.clone(),
                prompt: pending.prompt.clone(),
                model: pending.model.clone(),
                aspect_ratio: pending.aspect_ratio.clone(),
                file: None,
                remote: pending.remote.clone(),
                parameters: pending.parameters.clone(),
                input_images: pending.input_images.clone(),
                variant_group_id: pending.variant_group_id,
                title: pending.title.clone(),
                is_picked: false,
            })
            .collect();
        if images.is_empty() {
            return;
        }
        tracing::info!(count = images.len(), "saving images left unsaved");
        let created = pending.iter().map(|image| (image.id, image.created_at)).collect();
        self.save(images, created, Arc::new(|_, _| {}));
    }

    /// Moves the image's file to the Trash and takes it out of the Library. A file that isn't there is left to the
    /// Library; one not saved yet isn't tried again.
    pub async fn trash(&self, id: Uuid) -> Result<(), String> {
        let _ = self.pending.remove_pending(&HashSet::from([id])).await;
        let Some(material) = self.store.material(id).await.map_err(|error| error.to_string())? else {
            return Ok(());
        };
        let file = material
            .current
            .file_url
            .as_ref()
            .and_then(|url| url.to_file_path().ok());
        if let Some(file) = file {
            if !file.exists() {
                return Ok(());
            }
            tokio::task::spawn_blocking(move || trash::delete(&file).map_err(|error| error.to_string()))
                .await
                .map_err(|error| error.to_string())??;
        }
        self.store
            .remove_materials(&[id])
            .await
            .map_err(|error| error.to_string())
    }
}

/// Moves the download into `folder` as `<date> <prompt>.<ext>`, or with " 2", " 3"… when that name is taken.
async fn move_into(
    download: &Path,
    folder: &Path,
    image: &GeneratedImage,
    date: OffsetDateTime,
    offset: UtcOffset,
) -> Result<PathBuf, String> {
    let base = SavedImage::base_name(image.title.as_deref().unwrap_or(&image.prompt), date, offset);
    let extension = download.extension().map_or_else(
        || "png".to_owned(),
        |extension| extension.to_string_lossy().into_owned(),
    );
    let (download, folder) = (download.to_path_buf(), folder.to_path_buf());
    tokio::task::spawn_blocking(move || -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&folder)?;
        let file = unused_file(&folder, &base, &extension);
        if std::fs::rename(&download, &file).is_err() {
            // Another volume: copied, then the download removed with its folder.
            std::fs::copy(&download, &file)?;
        }
        Ok(file)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

/// `folder/<base>.<extension>`, or with " 2", " 3"… when that name is taken.
pub fn unused_file(folder: &Path, base: &str, extension: &str) -> PathBuf {
    let mut number = 1;
    loop {
        let name = if number == 1 {
            format!("{base}.{extension}")
        } else {
            format!("{base} {number}.{extension}")
        };
        let file = folder.join(name);
        if !file.exists() {
            return file;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_a_number() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(
            unused_file(folder.path(), "a", "png"),
            folder.path().join("a.png")
        );
        std::fs::write(folder.path().join("a.png"), "").unwrap();
        std::fs::write(folder.path().join("a 2.png"), "").unwrap();
        assert_eq!(
            unused_file(folder.path(), "a", "png"),
            folder.path().join("a 3.png")
        );
    }
}
