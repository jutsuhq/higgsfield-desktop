//! The model a job runs on.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::models::{
    KlingQuality, Mode, ModeLimits, ModelConstraintTable, Resolution, VideoControl, VideoModelCatalog,
    VideoModelDescriptor,
};
use crate::resources::models_string;
use crate::support::Seconds;
use crate::support::range::ClosedRange;

/// The model a job runs on, by its ID in `models.json` (`VideoModelCatalog`), which describes everything else.
/// Coded as the bare ID, so jobs, templates and settings stored when this was an enum read the same; an ID the
/// catalog doesn't have fails to decode, as an unknown case did.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = String))]
pub struct VideoModel(Cow<'static, str>);

impl fmt::Debug for VideoModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VideoModel({})", self.0)
    }
}

impl Serialize for VideoModel {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for VideoModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_raw(&raw).ok_or_else(|| serde::de::Error::custom(format!("Unknown video model \"{raw}\"")))
    }
}

impl VideoModel {
    /// Higgsfield's own; serves Motion, Restyle and Swap, and is the model every connection offers.
    pub const GENJUTSU: Self = Self(Cow::Borrowed("genjutsu"));
    /// Seedance 2.5.
    pub const SEEDANCE25: Self = Self(Cow::Borrowed("seedance25"));
    /// Kling 3.0 Motion Control.
    pub const KLING_MOTION_CONTROL: Self = Self(Cow::Borrowed("klingMotionControl"));

    /// `GENJUTSU`, for serde defaults.
    pub const fn genjutsu() -> Self {
        Self::GENJUTSU
    }

    /// The model of ID `raw`; `None` for an ID the catalog doesn't have.
    pub fn from_raw(raw: &str) -> Option<Self> {
        VideoModelCatalog::bundled()
            .get(raw)
            .map(|descriptor| Self(Cow::Owned(descriptor.id.clone())))
    }

    /// The model of ID `raw`, whether or not the catalog has it; such a model serves nothing.
    pub fn unchecked(raw: impl Into<String>) -> Self {
        Self(Cow::Owned(raw.into()))
    }

    /// Its ID.
    pub fn raw_value(&self) -> &str {
        &self.0
    }

    /// In the catalog's order.
    pub fn all() -> Vec<Self> {
        VideoModelCatalog::bundled()
            .descriptors()
            .iter()
            .map(|descriptor| Self(Cow::Owned(descriptor.id.clone())))
            .collect()
    }

    /// What the catalog says about it, or a descriptor that serves nothing for an ID it doesn't have.
    pub fn descriptor(&self) -> Cow<'static, VideoModelDescriptor> {
        VideoModelCatalog::bundled().get(&self.0).map_or_else(
            || Cow::Owned(VideoModelDescriptor::unknown(&self.0)),
            Cow::Borrowed,
        )
    }

    /// Its title, as the string catalog has it in English.
    pub fn title(&self) -> String {
        self.localized("title", &self.descriptor().title)
    }

    /// The picker's short name, shared by a family's models.
    pub fn family(&self) -> String {
        self.localized("family", &self.descriptor().family)
    }

    /// One line under the picker.
    pub fn detail(&self) -> String {
        self.localized("detail", &self.descriptor().detail)
    }

    /// The modes it serves, in the catalog's order.
    pub fn modes(&self) -> Vec<Mode> {
        self.descriptor().modes.clone()
    }

    /// The resolutions it renders at, lowest first.
    pub fn resolutions(&self) -> Vec<Resolution> {
        self.descriptor().resolutions.clone()
    }

    /// The resolutions it renders at in `mode`, lowest first: fewer than `resolutions` where the mode's schema
    /// says so (Genjutsu Lite's Object Swap, 720p and 1080p).
    pub fn resolutions_for(&self, mode: Mode) -> Vec<Resolution> {
        let descriptor = self.descriptor();
        descriptor
            .mode_resolutions
            .get(&mode)
            .cloned()
            .unwrap_or_else(|| descriptor.resolutions.clone())
    }

    /// `resolution` when the model renders at it, else the one it does nearest in frame height
    /// (`Resolution::pixel_height`), the higher on a tie: a form or a template kept at 480p asks Wan 2.7, which
    /// starts at 720p, for 720p; MiniMax H3 Max's 768p asks the others for 720p, and MiniMax H3's 2K for 1080p.
    pub fn fitted_any(&self, resolution: Resolution) -> Resolution {
        nearest(resolution, &self.resolutions())
    }

    /// `fitted_any` among the mode's own resolutions: Genjutsu Lite's Object Swap asked for 480p gets 720p.
    pub fn fitted(&self, resolution: Resolution, mode: Mode) -> Resolution {
        nearest(resolution, &self.resolutions_for(mode))
    }

    /// Whether it serves `mode`.
    pub fn serves(&self, mode: Mode) -> bool {
        self.descriptor().modes.contains(&mode)
    }

    /// Whether the REST API runs it, with an API key, in any of its modes; a model without endpoints runs on the
    /// account only (MiniMax H3 Max, which the public docs don't list).
    pub fn runs_with_api_key(&self) -> bool {
        self.modes()
            .into_iter()
            .any(|mode| self.runs_with_api_key_in(mode))
    }

    /// Whether the REST API runs it in `mode`: some endpoint applies to a job in that mode (`endpoint_id`).
    /// Genjutsu Lite has a Motion Transfer endpoint and none for Object Swap, which only the account runs.
    pub fn runs_with_api_key_in(&self, mode: Mode) -> bool {
        self.serves(mode)
            && self.resolutions_for(mode).into_iter().any(|resolution| {
                [true, false]
                    .into_iter()
                    .any(|has_media| !self.endpoint_id(mode, resolution, has_media, false).is_empty())
            })
    }

    /// Whether Create shows `control` for this model.
    pub fn declares(&self, control: VideoControl) -> bool {
        self.descriptor().controls.contains(&control)
    }

    /// Makes a clip from the prompt and images alone (`ClipWithoutSource`).
    pub fn makes_clips_without_source(&self) -> bool {
        self.declares(VideoControl::ClipWithoutSource)
    }

    /// A clip from frames may end on a second image (`frame_images`); Kling 4.0 Flash takes a start frame alone.
    pub fn takes_end_frame(&self) -> bool {
        self.descriptor().frame_images.upper() > 1
    }

    /// A model without a draft has a cheapest quality instead (Kling's Standard, whose 480p costs as much as 720p).
    pub fn offers_draft(&self) -> bool {
        self.declares(VideoControl::Draft)
    }

    /// The REST endpoint a job on this model runs on with an API key; also its key in `pricing.json`. The first
    /// of the catalog's `endpoints` keys that applies: the mode (Genjutsu's modes, Seedance's Edit and Extend),
    /// `frames` for a start and end frame (`uses_frames`), the quality (`standard` or `pro`) for a model with one,
    /// `reference` with a source or images and `text` without, then `default`. Empty when none is listed, which
    /// `Pricing` refuses at load.
    pub fn endpoint_id(
        &self,
        mode: Mode,
        resolution: Resolution,
        has_media: bool,
        uses_frames: bool,
    ) -> String {
        let descriptor = self.descriptor();
        let keys = [
            Some(mode.raw_value()),
            uses_frames.then_some("frames"),
            self.declares(VideoControl::Quality)
                .then(|| KlingQuality::from_resolution(resolution).raw_value()),
            Some(if has_media { "reference" } else { "text" }),
            Some("default"),
        ];
        keys.into_iter()
            .flatten()
            .find_map(|key| descriptor.endpoints.get(key).cloned())
            .unwrap_or_default()
    }

    /// What one job on this model takes in `mode`: the mode's limits with the catalog's over them. Seedance's
    /// 4–30 s is confirmed by `get_cost`, and Kling's one image by the server refusing more ("exactly one
    /// image_references entry is required"); a limit with no quote, schema or refusal behind it is Genjutsu's.
    /// Without a source (`has_source`) the prompt may stand alone, so no image is required, except by a model that
    /// voices a speech, whose images are the people who say it (Intone). `constraints` replaces these with what the
    /// server says, bound by bound, where it says something. Swift's defaults are `has_source: true` and the
    /// built-in table.
    pub fn limits(&self, mode: Mode, has_source: bool, constraints: &ModelConstraintTable) -> ModeLimits {
        let descriptor = self.descriptor();
        let mut limits = mode.limits();
        if let Some(duration) = descriptor.duration {
            limits.duration = duration;
        }
        if let Some(images) = descriptor.reference_images {
            limits.reference_images = images;
        }
        if let Some(server) = constraints.get(self, mode) {
            limits = server.applied(limits);
        }
        if !has_source && !self.declares(VideoControl::Speech) {
            limits.reference_images = ClosedRange::new(0, limits.reference_images.upper());
        }
        limits
    }

    /// `duration` inside the shorter window the model keeps for a job with references, a source or an end frame
    /// (`duration_with_references`): Gemini Omni Flash 1.2 makes 3–15 s from a prompt or a start frame alone, but
    /// 3–10 s from references or a start and an end frame, and Extend adds 3–10 s. Edit, whose output follows the
    /// source, keeps `duration`.
    pub fn duration(
        &self,
        duration: ClosedRange<Seconds>,
        mode: Mode,
        with_references: bool,
    ) -> ClosedRange<Seconds> {
        match self.descriptor().duration_with_references {
            Some(window) if with_references && mode != Mode::Edit => duration.clamped(window),
            _ => duration,
        }
    }

    /// The account's `generate_video` model ID for a job in `mode`; `None` where the account doesn't run it.
    pub fn account_model_id(&self, mode: Mode) -> Option<String> {
        self.descriptor().account_models.get(&mode).cloned()
    }

    /// The `<id>.<field>` key in `Models.xcstrings`, with the catalog's English as the fallback.
    fn localized(&self, field: &str, fallback: &str) -> String {
        if fallback.is_empty() {
            return String::new();
        }
        models_string(&format!("{}.{field}", self.0))
            .unwrap_or(fallback)
            .to_owned()
    }
}

fn nearest(resolution: Resolution, resolutions: &[Resolution]) -> Resolution {
    if resolutions.contains(&resolution) {
        return resolution;
    }
    let distance = |candidate: Resolution| (candidate.pixel_height() - resolution.pixel_height()).abs();
    let mut best: Option<Resolution> = None;
    for candidate in resolutions.iter().rev() {
        if best.is_none_or(|current| distance(*candidate) < distance(current)) {
            best = Some(*candidate);
        }
    }
    best.unwrap_or(resolution)
}
