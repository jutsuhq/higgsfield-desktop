//! Saved inputs to start new jobs from.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::models::{
    AudioOption, CharacterOrientation, CinemaControls, JobSpec, LocalImage, Mode, Resolution,
    SeedanceBitrate, VideoModel,
};
use crate::support::string_enum;
use crate::support::url_ext::last_path_component;
use crate::support::uuid_string;

string_enum! {
    /// Whether a template starts a video job or an image request. Templates saved before kinds decode as video.
    #[derive(Default)]
    pub enum TemplateKind {
        /// A video job.
        #[default]
        Video = "video",
        /// An image request.
        Image = "image",
    }
}

/// Saved inputs to start new jobs from: everything a `JobSpec` holds except the source and its range.
/// The references point at copies the app owns, so a template outlives the files it was made from.
/// An image template (`kind == Image`) starts an image request instead: only `image_model_id`,
/// `image_model_title`, `image_aspect_ratio`, `image_count`, `prompt` and `references` (the image to edit) apply.
///
/// Templates saved before `model` existed decode as Genjutsu, before the model parameters without them, and
/// before kinds as video templates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct JobTemplate {
    /// Its identity.
    pub id: Uuid,
    /// Its name.
    pub name: String,
    /// When it was saved.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
    /// Video or image.
    pub kind: TemplateKind,
    /// The mode.
    pub mode: Mode,
    /// The mode's default model (`Mode::default_model`) when the model doesn't serve `mode`.
    pub model: VideoModel,
    /// Ordered, like `JobSpec::references`.
    pub references: Vec<LocalImage>,
    /// Restyle only.
    #[serde(rename = "presetID", skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<Uuid>,
    /// The prompt.
    pub prompt: String,
    /// The resolution.
    pub resolution: Resolution,
    /// The sound.
    pub audio: AudioOption,
    /// Seedance only, as in `JobSpec`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<SeedanceBitrate>,
    /// Kling only, as in `JobSpec`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_orientation: Option<CharacterOrientation>,
    /// Cinema Studio only, as in `JobSpec`.
    pub cinema_controls: CinemaControls,
    /// `references` are a Seedance start frame and an optional end frame (`JobSpec::references_are_frames`).
    pub references_are_frames: bool,
    /// Image only: the image model's ID, for `ImageModel::id`.
    #[serde(rename = "imageModelID", skip_serializing_if = "Option::is_none")]
    pub image_model_id: Option<String>,
    /// Image only: the model title for display; the ID above is what applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_model_title: Option<String>,
    /// Image only: the frame, as "W:H".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_aspect_ratio: Option<String>,
    /// Image only: how many images one request makes.
    pub image_count: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTemplate {
    id: Uuid,
    name: String,
    #[serde(with = "crate::support::date::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(default)]
    kind: TemplateKind,
    mode: Mode,
    #[serde(default = "VideoModel::genjutsu")]
    model: VideoModel,
    references: Vec<LocalImage>,
    #[serde(default, rename = "presetID")]
    preset_id: Option<Uuid>,
    prompt: String,
    resolution: Resolution,
    audio: AudioOption,
    #[serde(default)]
    bitrate: Option<SeedanceBitrate>,
    #[serde(default)]
    character_orientation: Option<CharacterOrientation>,
    #[serde(default)]
    cinema_controls: CinemaControls,
    #[serde(default)]
    references_are_frames: bool,
    #[serde(default, rename = "imageModelID")]
    image_model_id: Option<String>,
    #[serde(default)]
    image_model_title: Option<String>,
    #[serde(default)]
    image_aspect_ratio: Option<String>,
    #[serde(default = "one")]
    image_count: i64,
}

const fn one() -> i64 {
    1
}

impl<'de> Deserialize<'de> for JobTemplate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredTemplate::deserialize(deserializer)?;
        Ok(Self {
            id: stored.id,
            name: stored.name,
            created_at: stored.created_at,
            kind: stored.kind,
            mode: stored.mode,
            model: stored.model,
            references: stored.references,
            preset_id: stored.preset_id,
            prompt: stored.prompt,
            resolution: stored.resolution,
            audio: stored.audio,
            bitrate: stored.bitrate,
            character_orientation: stored.character_orientation,
            cinema_controls: stored.cinema_controls,
            references_are_frames: stored.references_are_frames,
            image_model_id: stored.image_model_id,
            image_model_title: stored.image_model_title,
            image_aspect_ratio: stored.image_aspect_ratio,
            image_count: stored.image_count,
        })
    }
}

impl JobTemplate {
    /// A video template of `mode` with every other input at its default; `model` falls back to the mode's default
    /// model when it doesn't serve the mode. Set the other fields, then call `normalize` to apply the same rules.
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        created_at: OffsetDateTime,
        mode: Mode,
        model: VideoModel,
        resolution: Resolution,
    ) -> Self {
        let model = if model.serves(mode) {
            model
        } else {
            mode.default_model()
        };
        Self {
            id,
            name: name.into(),
            created_at,
            kind: TemplateKind::Video,
            mode,
            model,
            references: Vec::new(),
            preset_id: None,
            prompt: String::new(),
            resolution,
            audio: AudioOption::Original,
            bitrate: None,
            character_orientation: None,
            cinema_controls: CinemaControls::new(),
            references_are_frames: false,
            image_model_id: None,
            image_model_title: None,
            image_aspect_ratio: None,
            image_count: 1,
        }
    }

    /// Applies Swift's initializer rules after fields were set: the mode's default model for one that doesn't
    /// serve it, and no preset outside a mode that requires one.
    pub fn normalize(&mut self) {
        if !self.model.serves(self.mode) {
            self.model = self.mode.default_model();
        }
        if !self.mode.limits().requires_preset {
            self.preset_id = None;
        }
    }

    /// An image template: the model, frame, count, prompt and the image to edit.
    #[allow(clippy::too_many_arguments, reason = "mirrors the Swift initializer")]
    pub fn image(
        id: Uuid,
        name: impl Into<String>,
        created_at: OffsetDateTime,
        image_model_id: Option<String>,
        image_model_title: Option<String>,
        image_aspect_ratio: Option<String>,
        image_count: i64,
        prompt: impl Into<String>,
        references: Vec<LocalImage>,
    ) -> Self {
        let mut template = Self::new(
            id,
            name,
            created_at,
            Mode::MotionTransfer,
            VideoModel::genjutsu(),
            Resolution::P720,
        );
        template.kind = TemplateKind::Image;
        template.references = references;
        template.prompt = prompt.into();
        template.image_model_id = image_model_id;
        template.image_model_title = image_model_title;
        template.image_aspect_ratio = image_aspect_ratio;
        template.image_count = image_count;
        template
    }

    /// The inputs of `spec`, without its source.
    pub fn from_spec(id: Uuid, name: impl Into<String>, created_at: OffsetDateTime, spec: &JobSpec) -> Self {
        let mut template = Self::new(
            id,
            name,
            created_at,
            spec.mode,
            spec.model.clone(),
            spec.resolution,
        );
        template.references.clone_from(&spec.references);
        template.preset_id = spec.preset_id;
        template.prompt.clone_from(&spec.prompt);
        template.audio = spec.audio;
        template.bitrate = spec.bitrate;
        template.character_orientation = spec.character_orientation;
        template.cinema_controls = spec.cinema_controls.clone();
        template.references_are_frames = spec.uses_frames();
        template.normalize();
        template
    }
}

/// Where a template's reference copies go and which leftovers to remove. Decisions only; the caller does the I/O.
pub struct TemplateFiles;

impl TemplateFiles {
    /// The folder of one template's images, inside the templates folder.
    pub fn folder_name(template_id: Uuid) -> String {
        uuid_string(template_id)
    }

    /// File names for the copies, in order. The position prefix keeps two images with the same name apart and
    /// keeps the order visible in Finder.
    pub fn file_names(sources: &[Url]) -> Vec<String> {
        sources
            .iter()
            .enumerate()
            .map(|(index, url)| {
                let name = last_path_component(url);
                format!(
                    "{}-{}",
                    index + 1,
                    if name.is_empty() { "image" } else { name.as_str() }
                )
            })
            .collect()
    }

    /// Folders in the templates folder that belong to no saved template, e.g. after a save that failed once the
    /// images were copied.
    pub fn orphaned_folders(folder_names: &[String], keeping: &HashSet<Uuid>) -> Vec<String> {
        let kept: HashSet<String> = keeping.iter().map(|id| Self::folder_name(*id)).collect();
        let mut orphans: Vec<String> = folder_names
            .iter()
            .filter(|name| !kept.contains(*name))
            .cloned()
            .collect();
        orphans.sort();
        orphans
    }
}
