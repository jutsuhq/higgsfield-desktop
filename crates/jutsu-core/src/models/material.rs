//! The library's catalog entries: materials and their versions.

use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::models::{Job, PixelSize, Subtitles};
use crate::provider::ProviderKind;
use crate::support::string_enum;

/// One version of one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MaterialReference {
    /// The material.
    #[serde(rename = "materialID")]
    pub material_id: Uuid,
    /// The version.
    #[serde(rename = "versionID")]
    pub version_id: Uuid,
}

impl MaterialReference {
    /// The version `version_id` of `material_id`.
    pub const fn new(material_id: Uuid, version_id: Uuid) -> Self {
        Self {
            material_id,
            version_id,
        }
    }
}

string_enum! {
    /// What an input was to the version made from it.
    pub enum MaterialInputRole {
        /// The source video.
        SourceVideo = "sourceVideo",
        /// The start frame.
        StartFrame = "startFrame",
        /// The end frame.
        EndFrame = "endFrame",
        /// A reference image.
        Reference = "reference",
        /// The result an operation worked on.
        Parent = "parent",
    }
}

/// A version an item was made from, and in what role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MaterialInput {
    /// The input's version.
    pub source: MaterialReference,
    /// What it was.
    pub role: MaterialInputRole,
    /// Its place among the inputs of the same role.
    pub position: i64,
}

/// Identity belongs to a provider connection, not to an expiring download URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MaterialRemote {
    /// The connection kind.
    pub provider: ProviderKind,
    /// The connection's owner (`GenerationProvider::owner`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The provider's ID of the generation.
    pub id: String,
    /// Which of the generation's outputs.
    pub output: i64,
}

impl MaterialRemote {
    /// A unique key. Encoding components with their byte length avoids collisions between IDs containing
    /// separators.
    pub fn key(&self) -> String {
        let parts = [
            Some(self.provider.raw_value().to_owned()),
            self.owner.clone(),
            Some(self.id.clone()),
            Some(self.output.to_string()),
        ];
        parts
            .iter()
            .map(|part| {
                part.as_ref()
                    .map_or_else(|| "-".to_owned(), |text| format!("{}:{text}", text.len()))
            })
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// What made a version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MaterialGeneration {
    /// The prompt.
    pub prompt: String,
    /// The model's title.
    pub model: String,
    /// The exact known request, encoded as JSON. Absent for imported files and unknown provider parameters.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::support::codable::base64_option"
    )]
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub parameters: Option<Vec<u8>>,
    /// An agent's name for the images, shown instead of the prompt (`JobSpec::title_from`); `None` otherwise, and in
    /// materials saved before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

string_enum! {
    /// Whether a version made the cut. Separate from the star and from any client status: rejecting hides a version
    /// from the Library, it never deletes anything.
    #[derive(Default)]
    pub enum Culling {
        /// Not looked at yet.
        #[default]
        Unreviewed = "unreviewed",
        /// Kept.
        Kept = "kept",
        /// Rejected.
        Rejected = "rejected",
    }
}

/// Whether a version's file can be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionAvailability {
    /// The file is on this computer.
    Available,
    /// The file was here and is gone.
    Unavailable,
    /// Only the account holds it.
    Remote,
}

/// One version of a material: its file and what made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct MaterialVersion {
    /// Its identity.
    pub id: Uuid,
    /// When it was made.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// The file on this computer.
    #[serde(default, rename = "fileURL", skip_serializing_if = "Option::is_none")]
    pub file_url: Option<Url>,
    /// Where the provider serves it.
    #[serde(default, rename = "remoteURL", skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<Url>,
    /// What made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<MaterialGeneration>,
    /// What it was made from.
    #[serde(default)]
    pub inputs: Vec<MaterialInput>,
    /// "16:9".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// Its frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_size: Option<PixelSize>,
    /// Execution details used by the existing video actions. Library identity and stars belong to Material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<Box<Job>>,
    /// Each version is culled on its own: a new edit starts unreviewed.
    #[serde(default)]
    pub culling: Culling,
    /// Made from this version's own file; a new version starts without any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<Subtitles>,
}

impl MaterialVersion {
    /// A new, unreviewed version with nothing but its identity and date.
    pub fn new(id: Uuid, created_at: OffsetDateTime) -> Self {
        Self {
            id,
            created_at,
            file_url: None,
            remote_url: None,
            generation: None,
            inputs: Vec::new(),
            aspect_ratio: None,
            pixel_size: None,
            video: None,
            culling: Culling::Unreviewed,
            subtitles: None,
        }
    }

    /// Whether its file can be opened; `exists` says whether a file is at a path.
    pub fn availability(&self, exists: impl Fn(&Path) -> bool) -> VersionAvailability {
        let Some(file_url) = &self.file_url else {
            return VersionAvailability::Remote;
        };
        match file_url.to_file_path() {
            Ok(path) if exists(&path) => VersionAvailability::Available,
            _ => VersionAvailability::Unavailable,
        }
    }
}

string_enum! {
    /// Whether a material is a video or an image.
    pub enum MaterialKind {
        /// A video.
        Video = "video",
        /// An image.
        Image = "image",
    }
}

string_enum! {
    /// Where a material came from.
    #[derive(Default)]
    pub enum MaterialOrigin {
        /// Made or added in the app.
        #[default]
        Local = "local",
        /// Listed from the Higgsfield account.
        Higgsfield = "higgsfield",
    }
}

/// A catalog entry. Alternate generations are separate materials with a shared `variant_group_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Material {
    /// Its identity.
    pub id: Uuid,
    /// Video or image.
    pub kind: MaterialKind,
    /// Where it came from.
    pub origin: MaterialOrigin,
    /// The account's generation it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<MaterialRemote>,
    /// When it joined the catalog.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// Starred.
    pub is_starred: bool,
    /// Taken off the Library.
    #[serde(default)]
    pub is_hidden: bool,
    /// Its group of alternate generations.
    #[serde(default, rename = "variantGroupID", skip_serializing_if = "Option::is_none")]
    pub variant_group_id: Option<Uuid>,
    /// The variant chosen as the face of its group's card. At most one per group; with none, the grid picks.
    #[serde(default)]
    pub is_variant_primary: bool,
    /// The newest version.
    pub current: MaterialVersion,
    /// The versions before it, oldest first.
    #[serde(default)]
    previous_versions: Vec<MaterialVersion>,
}

/// Why a catalog change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum MaterialError {
    /// The version is already there.
    #[error("duplicateVersion")]
    DuplicateVersion,
    /// A stored record can't be read.
    #[error("invalidRecord")]
    InvalidRecord,
    /// The material or version isn't there.
    #[error("missingMaterial")]
    MissingMaterial,
    /// Taken off the Library, so it takes no new version.
    #[error("notListed")]
    NotListed,
}

impl Material {
    /// A local, unstarred material holding `version`.
    pub fn new(id: Uuid, kind: MaterialKind, created_at: OffsetDateTime, version: MaterialVersion) -> Self {
        Self {
            id,
            kind,
            origin: MaterialOrigin::Local,
            remote: None,
            created_at,
            is_starred: false,
            is_hidden: false,
            variant_group_id: None,
            is_variant_primary: false,
            current: version,
            previous_versions: Vec::new(),
        }
    }

    /// The current version.
    pub fn reference(&self) -> MaterialReference {
        MaterialReference::new(self.id, self.current.id)
    }

    /// The versions before the current one, oldest first.
    pub fn previous_versions(&self) -> &[MaterialVersion] {
        &self.previous_versions
    }

    /// Every version, oldest first.
    pub fn versions(&self) -> Vec<&MaterialVersion> {
        self.previous_versions
            .iter()
            .chain(std::iter::once(&self.current))
            .collect()
    }

    /// Makes `version` the current one.
    pub fn add_version(&mut self, version: MaterialVersion) -> Result<(), MaterialError> {
        if self.versions().iter().any(|existing| existing.id == version.id) {
            return Err(MaterialError::DuplicateVersion);
        }
        let previous = std::mem::replace(&mut self.current, version);
        self.previous_versions.push(previous);
        Ok(())
    }

    /// Library removal. An account result goes back to being listed from the account; a local one stops being
    /// listed but keeps its identity as a source of other versions.
    pub fn remove_from_library(&mut self) {
        if self.remote.is_none() || self.current.remote_url.is_none() {
            self.is_hidden = true;
            return;
        }
        self.current.file_url = None;
        if let Some(video) = self.current.video.as_mut() {
            video.output_file = None;
        }
    }

    /// Replaces the stored version of the same identity.
    pub fn update_version(&mut self, version: MaterialVersion) -> Result<(), MaterialError> {
        if version.id == self.current.id {
            self.current = version;
        } else if let Some(index) = self
            .previous_versions
            .iter()
            .position(|existing| existing.id == version.id)
        {
            self.previous_versions[index] = version;
        } else {
            return Err(MaterialError::MissingMaterial);
        }
        Ok(())
    }
}
