//! The resolution tiers the models render at.

use crate::support::string_enum;

string_enum! {
    /// The resolution tiers the models render at, lowest first.
    pub enum Resolution {
        /// Gemini Omni Flash 1.2's lowest tier.
        P360 = "360p",
        /// 480p.
        P480 = "480p",
        /// 720p.
        P720 = "720p",
        /// MiniMax H3 Max's higher tier.
        P768 = "768p",
        /// 1080p.
        P1080 = "1080p",
        /// MiniMax H3's only tier, spelled as its schemas spell it.
        K2 = "2K",
        /// Gemini Omni Flash 1.2's highest tier, spelled as its schema spells it.
        K4 = "4k",
    }
}

impl Resolution {
    /// What a model renders at unless `models.json` lists its own, and what Settings offers as a default.
    pub const STANDARD: [Self; 3] = [Self::P480, Self::P720, Self::P1080];

    /// The height of a 16:9 frame at this tier, in pixels; 2K is taken as 1440, the common 16:9 2K frame (MiniMax
    /// doesn't state its size), and 4K as 2160. `VideoModel::fitted` reads nearness by it.
    pub fn pixel_height(self) -> i64 {
        match self {
            Self::P360 => 360,
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P768 => 768,
            Self::P1080 => 1080,
            Self::K2 => 1440,
            Self::K4 => 2160,
        }
    }

    /// The nearest of `STANDARD`, for what takes only those (`reframe`'s output): 360p is 480p, 768p is 720p, 2K
    /// and 4K are 1080p.
    #[must_use]
    pub fn standardized(self) -> Self {
        match self {
            Self::P480 | Self::P720 | Self::P1080 => self,
            Self::P360 => Self::P480,
            Self::P768 => Self::P720,
            Self::K2 | Self::K4 => Self::P1080,
        }
    }
}
