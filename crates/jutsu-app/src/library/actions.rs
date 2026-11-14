//! Starring and deleting (`LibraryStarring.swift`, `LibraryDeletion.swift`, `MaterialVersions.earlierFiles`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::{GenerationLikes, Material, PortError};
use uuid::Uuid;

use super::item::LibraryItem;
use crate::connection::UserMessage;

/// Where a result's star is kept.
#[async_trait]
pub trait StarStore: Send + Sync {
    async fn set_starred(&self, is_starred: bool, id: Uuid) -> Result<(), String>;
}

/// Why a star didn't change.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StarError {
    /// The star couldn't be saved.
    #[error("{0}")]
    Store(String),
    /// Higgsfield refused the like; the star is back as it was.
    #[error(transparent)]
    Like(PortError),
}

/// The Library's star, which is also the result's like on Higgsfield when the connected account ran the job or has
/// it in its history.
#[derive(Clone)]
pub struct LibraryStarring {
    /// The connection's likes; `None` while disconnected or with an API key.
    pub likes: Option<Arc<dyn GenerationLikes>>,
    /// `GenerationProvider::owner` of the connection.
    pub owner: Option<String>,
}

impl LibraryStarring {
    /// Whether starring the item also likes it on the connected account.
    pub fn syncs_like(&self, item: &LibraryItem) -> bool {
        if item.like_id().is_none() || self.likes.is_none() {
            return false;
        }
        item.history_item.is_some() || item.job.owner == self.owner
    }

    /// Whether the star of a result can change here: a local result, or one in the account's history whose like it
    /// follows.
    pub fn can_star(&self, item: &LibraryItem) -> bool {
        item.history_item.is_none() || self.syncs_like(item)
    }

    /// Stars or unstars a result and likes it on Higgsfield to match. The local star is set at once and put back
    /// if Higgsfield refuses. `on_like` gets each like Higgsfield took.
    pub async fn set_starred(
        &self,
        is_starred: bool,
        item: &LibraryItem,
        store: &dyn StarStore,
        on_like: &mut (dyn FnMut(&str, bool) + Send),
    ) -> Result<(), StarError> {
        store
            .set_starred(is_starred, item.id())
            .await
            .map_err(StarError::Store)?;
        let (Some(likes), Some(id)) = (&self.likes, item.like_id()) else {
            return Ok(());
        };
        if !self.syncs_like(item) {
            return Ok(());
        }
        match likes.set_liked(id.clone(), is_starred).await {
            Ok(()) => {
                on_like(&id, is_starred);
                Ok(())
            }
            Err(error) => {
                store
                    .set_starred(!is_starred, item.id())
                    .await
                    .map_err(StarError::Store)?;
                Err(StarError::Like(error))
            }
        }
    }

    /// Stars or unstars each result that isn't so already, one after the other; the first like Higgsfield refuses
    /// stops the rest.
    pub async fn set_all_starred(
        &self,
        is_starred: bool,
        items: &[LibraryItem],
        store: &dyn StarStore,
        on_like: &mut (dyn FnMut(&str, bool) + Send),
    ) -> Result<(), StarError> {
        for item in items.iter().filter(|item| item.is_starred != is_starred) {
            self.set_starred(is_starred, item, store, on_like).await?;
        }
        Ok(())
    }
}

/// For a like Higgsfield refused, after the star was put back.
pub fn like_not_saved(error: &PortError) -> UserMessage {
    UserMessage::from_error(error, Some(jutsu_core::ProviderKind::Account)).unwrap_or_else(|| {
        UserMessage::new(
            "Can't update the like on Higgsfield",
            "The star is back as it was. Try again.",
            None,
        )
    })
}

/// Where a deleted result's file goes.
pub trait Trash: Send + Sync {
    fn file_exists(&self, path: &Path) -> bool;
    fn move_to_trash(&self, path: &Path) -> Result<(), String>;
}

/// The operating system's trash, through the `trash` crate.
pub struct SystemTrash;

impl Trash for SystemTrash {
    fn file_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|error| error.to_string())
    }
}

/// What Delete from Library did: the results whose files all went to the Trash, which leave the Library, and why
/// the others couldn't go.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Deletion {
    pub deleted: Vec<Uuid>,
    pub not_trashed: Vec<String>,
}

/// Moves each result's files to the Trash, its earlier versions first; a result whose file couldn't be moved stays,
/// so nothing is lost silently. `earlier_files`: the files of a result's earlier versions the app keeps.
pub fn trash_files(
    items: &[(Uuid, Option<PathBuf>)],
    trash: &dyn Trash,
    earlier_files: impl Fn(Uuid) -> Vec<PathBuf>,
) -> Deletion {
    let mut deletion = Deletion::default();
    'items: for (id, file) in items {
        for path in earlier_files(*id).iter().chain(file.iter()) {
            if !trash.file_exists(path) {
                continue;
            }
            if let Err(error) = trash.move_to_trash(path) {
                deletion.not_trashed.push(error);
                continue 'items;
            }
        }
        deletion.deleted.push(*id);
    }
    deletion
}

/// What to tell the user after a deletion; `None` when everything went. A failed save is reported before any Trash
/// failure.
pub fn deletion_message(deletion: &Deletion, saved: Result<(), String>) -> Option<UserMessage> {
    if let Err(error) = saved {
        return Some(UserMessage::new(
            "Can't remove the result from the Library",
            error,
            None,
        ));
    }
    match deletion.not_trashed.as_slice() {
        [] => None,
        [error] => Some(UserMessage::new(
            "Can't move the file to the Trash",
            error.clone(),
            None,
        )),
        [error, ..] => Some(UserMessage::new(
            &format!("Can't move {} files to the Trash", deletion.not_trashed.len()),
            format!("They stay in the Library. {error}"),
            None,
        )),
    }
}

/// What the selection bar, and the menu of a card in a selection, can do with the chosen results and images
/// (`LibraryBulkActions`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BulkActions {
    /// "3 selected".
    pub title: String,
    /// In the library: one only on Higgsfield has no file or record to delete.
    pub deletable_ids: Vec<String>,
    /// The ones whose star can change here.
    pub starrable_ids: Vec<String>,
    /// The star button stars them all unless every one is starred already.
    pub stars_all: bool,
    /// The Library menu's Star item: "Star", or "Unstar" when every one is starred.
    pub star_title: String,
    /// The one draft the Library menu's Render Final… opens; none for several, or for one only on Higgsfield
    /// (`LibraryMenuTargets.renderFinal`).
    pub render_final_id: Option<String>,
    /// Compare, for two chosen images with at least one here: the ones it picks, a missing file left out
    /// (`LibraryImageSelectionBar`). Empty when the bar has no Compare.
    pub compare_ids: Vec<String>,
    /// The downloaded results, for Show in File Manager, Copy Videos and Share…, which are offered only with one
    /// (`LibraryBulkActions.files`).
    pub file_ids: Vec<String>,
}

/// Where a chosen image's file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFile {
    Here,
    /// Listed, but the file is gone or on a drive that isn't connected.
    Missing,
    OnHiggsfield,
}

/// One chosen result or image: its ID, whether it is only on Higgsfield, whether its star can change here and
/// whether it is starred.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one per question the selection asks of a result"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkEntry {
    pub id: Uuid,
    pub needs_import: bool,
    pub can_star: bool,
    pub is_starred: bool,
    /// A draft here, not one only on Higgsfield (`finals::can_render_final`).
    pub can_render_final: bool,
    /// Where the file is, for an image; `None` for a result.
    pub image: Option<ImageFile>,
    /// A result downloaded into the library, with its file.
    pub has_file: bool,
}

impl BulkActions {
    pub fn new(entries: &[BulkEntry]) -> Self {
        let starrable: Vec<&BulkEntry> = entries.iter().filter(|entry| entry.can_star).collect();
        let stars_all = !starrable.iter().all(|entry| entry.is_starred);
        let render_final_id = match entries {
            [only] if only.can_render_final => Some(only.id.to_string()),
            _ => None,
        };
        Self {
            title: format!("{} selected", entries.len()),
            deletable_ids: entries
                .iter()
                .filter(|entry| !entry.needs_import)
                .map(|entry| entry.id.to_string())
                .collect(),
            starrable_ids: starrable.iter().map(|entry| entry.id.to_string()).collect(),
            stars_all,
            star_title: if stars_all { "Star" } else { "Unstar" }.to_owned(),
            render_final_id,
            compare_ids: compare_ids(entries),
            file_ids: entries
                .iter()
                .filter(|entry| entry.has_file)
                .map(|entry| entry.id.to_string())
                .collect(),
        }
    }
}

/// Compare on the selection bar of two images, one of them at least here: each that can be picked, as Compare… picks
/// it; one whose file is missing is skipped.
fn compare_ids(entries: &[BulkEntry]) -> Vec<String> {
    let images: Vec<(Uuid, ImageFile)> = entries
        .iter()
        .filter_map(|entry| entry.image.map(|file| (entry.id, file)))
        .collect();
    let offered =
        entries.len() == 2 && images.len() == 2 && images.iter().any(|(_, file)| *file == ImageFile::Here);
    if !offered {
        return Vec::new();
    }
    images
        .into_iter()
        .filter(|(_, file)| *file != ImageFile::Missing)
        .map(|(id, _)| id.to_string())
        .collect()
}

/// Use as Reference's notice when Create couldn't take every image, as dropping them on Create titles it.
pub const REFERENCES_NOT_ADDED: &str = "Some files weren't added";

/// The alert after images failed to move to the Trash (`LibraryView.trashImages`).
pub const IMAGE_TRASH_FAILED: &str = "Couldn't move the images to the Trash";

/// What to tell the user after deleting images: any failure under the images' own title.
pub fn image_deletion_message(deletion: &Deletion, saved: Result<(), String>) -> Option<UserMessage> {
    deletion_message(deletion, saved)
        .map(|message| UserMessage::new(IMAGE_TRASH_FAILED, message.message, None))
}

/// The question Move to Trash… asks before images go, with its button (`LibraryView`'s image trash dialog).
pub fn image_trash_confirmation(count: usize) -> (String, String) {
    (
        format!("Move {count} images to the Trash?"),
        "Their earlier versions made in Jutsu Studio go too. You can put them back from the Trash in your file manager."
            .to_owned(),
    )
}

/// The confirmation shown before results are deleted.
pub fn delete_confirmation(count: usize) -> (String, String) {
    if count == 1 {
        (
            "Delete from library?".to_owned(),
            "The video file and its earlier versions made in Jutsu Studio move to the Trash, and its history entry is removed."
                .to_owned(),
        )
    } else {
        (
            format!("Delete {count} results from library?"),
            "The video files and their earlier versions made in Jutsu Studio move to the Trash, and their history entries are removed."
                .to_owned(),
        )
    }
}

/// Remove from Library…'s question for images whose files are gone (`LibraryView.pendingImageRemoval`).
pub fn image_removal_title(count: usize) -> String {
    format!("Remove {count} images from the library?")
}

/// The files of the material's other versions that the app keeps (`owns`), once each.
pub fn earlier_files(material: &Material, owns: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let path = |version: &jutsu_core::MaterialVersion| {
        version.file_url.as_ref().and_then(|url| url.to_file_path().ok())
    };
    let current = path(&material.current);
    let mut seen = HashSet::new();
    material
        .versions()
        .into_iter()
        .filter_map(path)
        .filter(|file| Some(file) != current.as_ref() && owns(file) && seen.insert(file.clone()))
        .collect()
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
