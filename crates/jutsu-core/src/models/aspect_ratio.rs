//! The frame of a clip made without a source.

use serde::{Deserialize, Serialize};

use crate::models::PixelSize;

/// The frame of a clip made without a source video, as `generate_video` names it; a clip made from a source keeps
/// the source's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum AspectRatio {
    /// The model's choice: the reference image's frame when there is one.
    #[serde(rename = "auto")]
    Auto,
    /// 21:9.
    #[serde(rename = "21:9")]
    Cinema,
    /// 16:9.
    #[serde(rename = "16:9")]
    Landscape,
    /// 4:3.
    #[serde(rename = "4:3")]
    Classic,
    /// 1:1.
    #[serde(rename = "1:1")]
    Square,
    /// 3:4.
    #[serde(rename = "3:4")]
    ClassicPortrait,
    /// 9:16.
    #[serde(rename = "9:16")]
    Portrait,
}

impl AspectRatio {
    /// Every frame, in order.
    pub const ALL: [Self; 7] = [
        Self::Auto,
        Self::Cinema,
        Self::Landscape,
        Self::Classic,
        Self::Square,
        Self::ClassicPortrait,
        Self::Portrait,
    ];

    /// The stored and sent name: "auto", "16:9"…
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cinema => "21:9",
            Self::Landscape => "16:9",
            Self::Classic => "4:3",
            Self::Square => "1:1",
            Self::ClassicPortrait => "3:4",
            Self::Portrait => "9:16",
        }
    }

    /// The frame named `raw`.
    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|ratio| ratio.raw_value() == raw)
    }

    /// Width over height; `None` for `Auto`, whose frame only the result shows.
    pub fn width_over_height(self) -> Option<f64> {
        let sides: Vec<f64> = self
            .raw_value()
            .split(':')
            .filter_map(|side| side.parse().ok())
            .collect();
        match sides.as_slice() {
            [width, height] if *height > 0.0 => Some(width / height),
            _ => None,
        }
    }

    /// The fixed frame among `candidates` closest in shape to `size` (never `Auto`); `None` for a size without both
    /// sides. A start frame's ratio, which the account's Seedance doesn't derive itself: sent none, it made a 16:9
    /// clip from a 9:16 start frame in a live run on 2026-10-15.
    pub fn nearest(size: PixelSize, candidates: &[Self]) -> Option<Self> {
        if !size.is_valid() {
            return None;
        }
        let target = (size.width as f64 / size.height as f64).ln();
        let mut best: Option<(Self, f64)> = None;
        for ratio in candidates {
            let Some(shape) = ratio.width_over_height() else {
                continue;
            };
            let distance = (shape.ln() - target).abs();
            if best.is_none_or(|(_, closest)| distance < closest) {
                best = Some((*ratio, distance));
            }
        }
        best.map(|(ratio, _)| ratio)
    }
}
