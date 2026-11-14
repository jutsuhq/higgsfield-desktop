//! A named set of exact catalog versions, exported in several outputs at once.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::models::{
    CropAspect, CropFrame, DeliveryOutput, DeliveryPreset, ExportPreset, ImageExportPreset, ImageFormat,
    Material, MaterialKind, MaterialReference,
};
use crate::support::string_enum;
use crate::support::url_ext::{deleting_path_extension, last_path_component};

/// One version in a delivery set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DeliveryItem {
    /// The exact version.
    pub reference: MaterialReference,
    /// Video or image.
    pub kind: MaterialKind,
    /// The item's part of the file names: its file's name when it joined the set.
    pub name: String,
    /// Frame adjustments by `CropAspect::key`; an aspect without one is cropped centred.
    #[serde(default)]
    pub frames: BTreeMap<String, CropFrame>,
}

impl DeliveryItem {
    /// An item cropped centred for every aspect.
    pub fn new(reference: MaterialReference, kind: MaterialKind, name: impl Into<String>) -> Self {
        Self {
            reference,
            kind,
            name: name.into(),
            frames: BTreeMap::new(),
        }
    }

    /// The frame for an output of `aspect`; `None` for an output that doesn't crop.
    pub fn frame(&self, aspect: Option<CropAspect>) -> Option<CropFrame> {
        aspect.map(|aspect| {
            self.frames
                .get(&aspect.key())
                .copied()
                .unwrap_or(CropFrame::CENTERED)
        })
    }
}

/// The folder a set was last exported to. The macOS app also keeps a security-scoped bookmark of it, which has no
/// equivalent here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DeliveryFolder {
    /// The folder.
    pub url: Url,
}

/// Whether a set has been exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverySetStatus {
    /// Not exported yet.
    NotExported,
    /// Exported, when and where.
    Exported(OffsetDateTime, Url),
}

/// A named set of exact catalog versions, images and videos, made from a Library selection and exported in several
/// outputs at once. Once a file of it has been exported the set is frozen: its versions never change, so a later
/// edit of an item never replaces what was delivered. `next_round` starts a new set from the items' newest versions.
/// Frames, outputs, the note and the review link stay editable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DeliverySet {
    /// Its identity.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// When it was made.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    items: Vec<DeliveryItem>,
    /// The outputs every item is written in.
    pub outputs: Vec<DeliveryOutput>,
    /// A note for whoever receives it.
    pub note: String,
    /// Text only: the app never opens or uploads to it.
    pub review_link: String,
    results: Vec<DeliveryResult>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::date::rfc3339_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    exported_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<DeliveryFolder>,
}

/// Why a delivery set change or export was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum DeliverySetError {
    /// The set has been exported, so its versions can't change.
    #[error("frozen")]
    Frozen,
    /// The item's file isn't on this computer: removed, on a drive that isn't connected, or only on the account.
    #[error("sourceUnavailable")]
    SourceUnavailable,
    /// The output writes or burns in subtitles, and the item's version has none.
    #[error("noSubtitles")]
    NoSubtitles,
}

impl DeliverySet {
    /// A new, unexported set of `items` (one per version, in order).
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        created_at: OffsetDateTime,
        items: Vec<DeliveryItem>,
        outputs: Vec<DeliveryOutput>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            created_at,
            items: unique(items),
            outputs,
            note: String::new(),
            review_link: String::new(),
            results: Vec::new(),
            exported_at: None,
            folder: None,
        }
    }

    /// A full-size JPEG for images and an H.264 file for videos.
    pub fn default_outputs() -> Vec<DeliveryOutput> {
        vec![
            DeliveryOutput::new(DeliveryPreset::Image(ImageExportPreset::new(ImageFormat::Jpeg))),
            DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264)),
        ]
    }

    /// The current version of each material, in order, named after its file or else its prompt.
    pub fn items_from(materials: &[Material]) -> Vec<DeliveryItem> {
        materials
            .iter()
            .map(|material| {
                let version = &material.current;
                let file_name = version
                    .file_url
                    .as_ref()
                    .map(|url| deleting_path_extension(&last_path_component(url)).to_owned());
                let name = file_name
                    .or_else(|| {
                        version
                            .generation
                            .as_ref()
                            .map(|generation| generation.prompt.clone())
                    })
                    .unwrap_or_default();
                DeliveryItem::new(material.reference(), material.kind, name)
            })
            .collect()
    }

    /// The versions, in order.
    pub fn items(&self) -> &[DeliveryItem] {
        &self.items
    }

    /// The newest result of each item and output exported so far.
    pub fn results(&self) -> &[DeliveryResult] {
        &self.results
    }

    /// When a file of it was first exported.
    pub fn exported_at(&self) -> Option<OffsetDateTime> {
        self.exported_at
    }

    /// The folder it was last exported to.
    pub fn folder(&self) -> Option<&DeliveryFolder> {
        self.folder.as_ref()
    }

    /// Exported at least once.
    pub fn is_frozen(&self) -> bool {
        self.exported_at.is_some()
    }

    /// Whether and where it was exported.
    pub fn status(&self) -> DeliverySetStatus {
        match (self.exported_at, &self.folder) {
            (Some(at), Some(folder)) => DeliverySetStatus::Exported(at, folder.url.clone()),
            _ => DeliverySetStatus::NotExported,
        }
    }

    /// Replaces the set's versions; refused once the set is frozen.
    pub fn set_items(&mut self, items: Vec<DeliveryItem>) -> Result<(), DeliverySetError> {
        if self.is_frozen() {
            return Err(DeliverySetError::Frozen);
        }
        self.items = unique(items);
        Ok(())
    }

    /// Moves an item's crop for one aspect. Allowed on a frozen set: the version stays, and the item's files for
    /// outputs of that aspect become pending again.
    pub fn set_frame(&mut self, frame: CropFrame, reference: MaterialReference, aspect: CropAspect) {
        let Some(item) = self.items.iter_mut().find(|item| item.reference == reference) else {
            return;
        };
        if frame == CropFrame::CENTERED {
            item.frames.remove(&aspect.key());
        } else {
            item.frames.insert(aspect.key(), frame);
        }
    }

    /// The newest result of `item` in `output`.
    pub fn result(&self, item: &DeliveryItem, output: &DeliveryOutput) -> Option<&DeliveryResult> {
        self.results
            .iter()
            .find(|result| result.item == item.reference && result.output.id == output.id)
    }

    /// Notes what a run did. The set freezes on its first exported file, and keeps the folder it went to.
    pub fn record(&mut self, new: Vec<DeliveryResult>, folder: DeliveryFolder, date: OffsetDateTime) {
        let any_done = new
            .iter()
            .any(|result| result.status == DeliveryResultStatus::Done);
        for result in new {
            self.results
                .retain(|kept| !(kept.item == result.item && kept.output.id == result.output.id));
            self.results.push(result);
        }
        if !(self.is_frozen() || any_done) {
            return;
        }
        self.exported_at = Some(date);
        self.folder = Some(folder);
    }

    /// A new, unexported set of the same materials at `current_version` (their newest version), with the same
    /// outputs and frames. A material that is gone keeps the version this set holds.
    pub fn next_round(
        &self,
        name: impl Into<String>,
        date: OffsetDateTime,
        current_version: impl Fn(Uuid) -> Option<MaterialReference>,
    ) -> Self {
        let next = self
            .items
            .iter()
            .map(|item| {
                let mut item = item.clone();
                item.reference = current_version(item.reference.material_id).unwrap_or(item.reference);
                item
            })
            .collect();
        Self::new(Uuid::new_v4(), name, date, next, self.outputs.clone())
    }
}

/// One item per version, in the order given.
fn unique(items: Vec<DeliveryItem>) -> Vec<DeliveryItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.reference))
        .collect()
}

string_enum! {
    /// How one item in one output went.
    pub enum DeliveryResultStatus {
        /// Written.
        Done = "done",
        /// Failed.
        Failed = "failed",
        /// Skipped.
        Skipped = "skipped",
    }
}

/// What one export did with one item in one output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct DeliveryResult {
    /// The version.
    pub item: MaterialReference,
    /// The output as it was when the file was written, so a changed preset makes the file pending again.
    pub output: DeliveryOutput,
    /// How it went.
    pub status: DeliveryResultStatus,
    /// The file written, for `Done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// The crop it was written with; `None` for an output that doesn't crop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<CropFrame>,
    /// Why it failed or was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl DeliveryResult {
    /// A result with no file, frame or message.
    pub fn new(item: MaterialReference, output: DeliveryOutput, status: DeliveryResultStatus) -> Self {
        Self {
            item,
            output,
            status,
            file_name: None,
            frame: None,
            message: None,
        }
    }
}
