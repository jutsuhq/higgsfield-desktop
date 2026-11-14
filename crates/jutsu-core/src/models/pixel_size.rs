//! Frame sizes in pixels and how people name their shapes.

use serde::{Deserialize, Serialize};

/// A frame's width and height in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PixelSize {
    /// Pixels across.
    pub width: i64,
    /// Pixels down.
    pub height: i64,
}

/// Frames people name, landscape to portrait.
const NAMED_RATIOS: [(&str, f64); 10] = [
    ("21:9", 21.0 / 9.0),
    ("16:9", 16.0 / 9.0),
    ("3:2", 3.0 / 2.0),
    ("4:3", 4.0 / 3.0),
    ("5:4", 5.0 / 4.0),
    ("1:1", 1.0),
    ("4:5", 4.0 / 5.0),
    ("3:4", 3.0 / 4.0),
    ("2:3", 2.0 / 3.0),
    ("9:16", 9.0 / 16.0),
];

impl PixelSize {
    /// `width` × `height`.
    pub const fn new(width: i64, height: i64) -> Self {
        Self { width, height }
    }

    /// Sides of at least one pixel; `Validation::check` rejects anything else.
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// 0 for an invalid size, `i64::MAX` when the product doesn't fit.
    pub fn pixel_count(&self) -> i64 {
        if !self.is_valid() {
            return 0;
        }
        self.width.checked_mul(self.height).unwrap_or(i64::MAX)
    }

    /// The frame as people name it, "4:3"; `None` for an invalid size. Models round their sides (1460×1100 is
    /// 73:55), so a size within 2 % of a named frame gets its name; another keeps its reduced ratio while both terms
    /// are at most 32, and otherwise gets the nearest named frame.
    pub fn aspect_label(&self) -> Option<String> {
        if !self.is_valid() {
            return None;
        }
        let shape = (self.width as f64 / self.height as f64).ln();
        let distance = |value: f64| (value.ln() - shape).abs();
        let mut nearest = NAMED_RATIOS[0];
        for candidate in NAMED_RATIOS {
            if distance(candidate.1) < distance(nearest.1) {
                nearest = candidate;
            }
        }
        if distance(nearest.1) < 0.02 {
            return Some(nearest.0.to_owned());
        }
        let (mut a, mut b) = (self.width, self.height);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let (w, h) = (self.width / a, self.height / a);
        Some(if w.max(h) <= 32 {
            format!("{w}:{h}")
        } else {
            nearest.0.to_owned()
        })
    }

    /// `aspect_label` for a stored "W:H"; any other text as it is.
    pub fn aspect_label_of(ratio: &str) -> String {
        let sides: Vec<i64> = ratio.split(':').filter_map(|side| side.parse().ok()).collect();
        match sides.as_slice() {
            [width, height] => Self::new(*width, *height)
                .aspect_label()
                .unwrap_or_else(|| ratio.to_owned()),
            _ => ratio.to_owned(),
        }
    }
}
