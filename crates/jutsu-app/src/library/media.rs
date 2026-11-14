//! What the Library grid shows (`LibraryMedia.swift`, `MaterialLibrary.swift`): videos, images made here, and
//! images only on the Higgsfield account, read from the catalog.

use jutsu_core::{
    HistoryImageItem, HistoryItem, Material, MaterialKind, PixelSize, ProviderKind, RemoteInputs, SavedImage,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::item::LibraryItem;

/// The kind tabs: videos, images, or both, interleaved by date.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LibraryKind {
    #[default]
    All,
    Video,
    Image,
}

/// Where the grid's results come from: this computer, or the Higgsfield account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LibraryOrigin {
    #[default]
    Local,
    Higgsfield,
}

/// One row of the grid: a video result, an image made here, or an image still only on the account.
#[allow(
    clippy::large_enum_variant,
    reason = "rows live for one snapshot, and most are videos"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum MediaRow {
    Video(LibraryItem),
    LocalImage(SavedImage),
    RemoteImage(HistoryImageItem),
}

impl MediaRow {
    /// The material's ID.
    pub fn id(&self) -> Uuid {
        match self {
            Self::Video(item) => item.id(),
            Self::LocalImage(image) => image.id,
            Self::RemoteImage(image) => image.job_id,
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self, Self::Video(_))
    }

    /// When a video finished, when an image was made.
    pub fn created_at(&self) -> OffsetDateTime {
        match self {
            Self::Video(item) => item.finished_at(),
            Self::LocalImage(image) => image.created_at,
            Self::RemoteImage(image) => image.created_at,
        }
    }

    /// Starred here, or liked on Higgsfield: the star and the like are one.
    pub fn is_starred(&self) -> bool {
        match self {
            Self::Video(item) => item.is_starred,
            Self::LocalImage(image) => image.is_starred,
            Self::RemoteImage(image) => image.is_liked,
        }
    }

    /// What the search looks through.
    pub fn search_text(&self) -> String {
        match self {
            Self::Video(item) => format!("{} {}", item.spec().prompt, item.title()),
            Self::LocalImage(image) => format!(
                "{} {} {}",
                image.prompt,
                image.model,
                image.title.as_deref().unwrap_or("")
            ),
            Self::RemoteImage(image) => format!("{} {}", image.prompt, image.model),
        }
    }

    /// The image model's name; `None` for videos.
    pub fn image_model(&self) -> Option<&str> {
        match self {
            Self::Video(_) => None,
            Self::LocalImage(image) => Some(&image.model),
            Self::RemoteImage(image) => Some(&image.model),
        }
    }

    /// "16:9" for images; `None` for videos and when unknown.
    pub fn aspect_ratio(&self) -> Option<String> {
        match self {
            Self::Video(_) => None,
            // Saved before labels were rounded, a stored ratio can still be "73:55".
            Self::LocalImage(image) => image.aspect_ratio.as_deref().map(PixelSize::aspect_label_of),
            Self::RemoteImage(image) => image.aspect_ratio(),
        }
    }

    pub fn video(&self) -> Option<&LibraryItem> {
        match self {
            Self::Video(item) => Some(item),
            _ => None,
        }
    }
}

/// How many rows each kind tab holds, after every filter but the kind itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KindCounts {
    pub videos: i32,
    pub images: i32,
    pub all: i32,
}

impl KindCounts {
    pub fn new(videos: usize, images: usize) -> Self {
        let count = |value: usize| i32::try_from(value).unwrap_or(i32::MAX);
        Self {
            videos: count(videos),
            images: count(images),
            all: count(videos + images),
        }
    }

    pub fn count(&self, kind: LibraryKind) -> i32 {
        match kind {
            LibraryKind::All => self.all,
            LibraryKind::Video => self.videos,
            LibraryKind::Image => self.images,
        }
    }
}

/// The Library row of a material; `None` for one taken off the Library or without anything to show.
pub fn material_row(material: &Material) -> Option<MediaRow> {
    if material.is_hidden {
        return None;
    }
    let version = &material.current;
    if material.kind == MaterialKind::Video
        && let Some(job) = &version.video
    {
        let mut job = (**job).clone();
        job.output_file.clone_from(&version.file_url);
        if version.file_url.is_some() {
            return Some(MediaRow::Video(LibraryItem::new(job, material.is_starred)));
        }
        let (remote, url) = (material.remote.as_ref()?, version.remote_url.clone()?);
        let mut history = HistoryItem::new(
            remote.id.clone(),
            job.spec.mode,
            job.spec.model.clone(),
            material.created_at,
            url,
            job.inputs.clone().unwrap_or(RemoteInputs {
                video: None,
                images: Vec::new(),
                audio: None,
            }),
        );
        history.job_id = material.id;
        history.is_liked = material.is_starred;
        history.duration = Some(job.output_duration());
        history.pixel_size = job.output_media.as_ref().map(|media| media.pixel_size);
        history.resolution = Some(job.spec.resolution);
        history.prompt = version
            .generation
            .as_ref()
            .map(|generation| generation.prompt.clone())
            .unwrap_or_default();
        history.is_native_draft = job.remote.as_ref().is_some_and(|remote| remote.is_native_draft);
        history.draft_job_id.clone_from(&job.spec.draft_job_id);
        let mut item = LibraryItem::from_history(history);
        item.job.owner.clone_from(&job.owner);
        return Some(MediaRow::Video(item));
    }
    if let Some(image) = saved_image(material) {
        return Some(MediaRow::LocalImage(image));
    }
    if material.kind != MaterialKind::Image {
        return None;
    }
    let (remote, url) = (material.remote.as_ref()?, version.remote_url.clone()?);
    let generation = version.generation.as_ref();
    Some(MediaRow::RemoteImage(HistoryImageItem {
        id: remote.id.clone(),
        job_id: material.id,
        prompt: generation
            .map(|generation| generation.prompt.clone())
            .unwrap_or_default(),
        model: generation
            .map(|generation| generation.model.clone())
            .unwrap_or_default(),
        created_at: material.created_at,
        is_liked: material.is_starred,
        output_url: url,
        pixel_size: version.pixel_size,
    }))
}

/// The image of a material with a file.
pub fn saved_image(material: &Material) -> Option<SavedImage> {
    if material.kind != MaterialKind::Image {
        return None;
    }
    let version = &material.current;
    let generation = version.generation.as_ref();
    Some(SavedImage {
        id: material.id,
        file_url: version.file_url.clone()?,
        prompt: generation
            .map(|generation| generation.prompt.clone())
            .unwrap_or_default(),
        model: generation
            .map(|generation| generation.model.clone())
            .unwrap_or_default(),
        aspect_ratio: version.aspect_ratio.clone(),
        created_at: material.created_at,
        remote_url: version.remote_url.clone(),
        is_starred: material.is_starred,
        title: generation.and_then(|generation| generation.title.clone()),
    })
}

/// The origin's materials: those with a file here, or the connected account's.
pub fn in_origin(material: &Material, origin: LibraryOrigin, owner: Option<&str>) -> bool {
    match origin {
        LibraryOrigin::Local => material.current.file_url.is_some(),
        LibraryOrigin::Higgsfield => material.remote.as_ref().is_some_and(|remote| {
            remote.provider == ProviderKind::Account && remote.owner.as_deref() == owner
        }),
    }
}
