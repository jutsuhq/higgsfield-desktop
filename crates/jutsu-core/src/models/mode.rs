//! What a job does with its source.

use crate::models::VideoModel;
use crate::rules::Validation;
use crate::support::Seconds;
use crate::support::range::ClosedRange;
use crate::support::string_enum;

string_enum! {
    /// What a job does.
    pub enum Mode {
        /// The character of the reference image moves as the source video does.
        MotionTransfer = "motionTransfer",
        /// The source restyled with a preset.
        Restyle = "restyle",
        /// An object of the source replaced with the reference image's.
        ObjectSwap = "objectSwap",
        /// Seedance changes an existing clip as the prompt says; the output follows the source's length and frame.
        Edit = "edit",
        /// Seedance continues an existing clip past its last frame.
        Extend = "extend",
    }
}

impl Mode {
    /// Genjutsu's limits, and Seedance's for Edit and Extend; `VideoModel::limits` gives each model's.
    pub fn limits(self) -> ModeLimits {
        match self {
            Self::MotionTransfer | Self::ObjectSwap => ModeLimits::new(ClosedRange::new(1, 8), false),
            Self::Restyle => ModeLimits::new(ClosedRange::new(0, 5), true),
            Self::Edit | Self::Extend => ModeLimits::new(ClosedRange::new(0, 8), false),
        }
    }

    /// Edit and Extend: operations on an existing clip, which need the source and a prompt (both REST schemas
    /// require `prompt` with at least one character).
    pub fn is_video_operation(self) -> bool {
        matches!(self, Self::Edit | Self::Extend)
    }

    /// The model a job in this mode runs on when the picked one doesn't serve it: the first in the catalog that
    /// serves it (Genjutsu, and Seedance for Edit and Extend).
    pub fn default_model(self) -> VideoModel {
        VideoModel::all()
            .into_iter()
            .find(|model| model.serves(self))
            .unwrap_or_else(VideoModel::genjutsu)
    }
}

/// The inputs one job of a mode takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeLimits {
    /// How many reference images.
    pub reference_images: ClosedRange<usize>,
    /// Restyle needs a preset.
    pub requires_preset: bool,
    /// Seconds of source one job takes; for Extend also the range of seconds it adds.
    pub duration: ClosedRange<Seconds>,
}

impl ModeLimits {
    /// The limits with `Validation::DURATION_RANGE`.
    pub fn new(reference_images: ClosedRange<usize>, requires_preset: bool) -> Self {
        Self {
            reference_images,
            requires_preset,
            duration: Validation::DURATION_RANGE,
        }
    }
}
