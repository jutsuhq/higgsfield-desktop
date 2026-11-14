//! Kling's quality mode.

use crate::models::Resolution;

/// Kling's quality mode, which it has instead of a resolution: Standard renders 720p, Pro 1080p. A job keeps it in
/// `JobSpec::resolution`, so any resolution below 1080p, such as a 480p draft made before the picker, is Standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KlingQuality {
    /// 720p.
    Standard,
    /// 1080p.
    Pro,
}

impl KlingQuality {
    /// Every quality, in order.
    pub const ALL: [Self; 2] = [Self::Standard, Self::Pro];

    /// The quality a job stored at `resolution` has.
    pub fn from_resolution(resolution: Resolution) -> Self {
        if resolution == Resolution::P1080 {
            Self::Pro
        } else {
            Self::Standard
        }
    }

    /// "standard" or "pro".
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Pro => "pro",
        }
    }

    /// The resolution a job at this quality is stored with.
    pub fn resolution(self) -> Resolution {
        match self {
            Self::Standard => Resolution::P720,
            Self::Pro => Resolution::P1080,
        }
    }
}
