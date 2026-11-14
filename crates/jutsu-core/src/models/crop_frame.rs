//! Where a delivery output crops each item.

use serde::{Deserialize, Serialize};

use crate::models::PixelSize;

/// The shape a delivery output crops to, e.g. 4:5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct CropAspect {
    /// The width term.
    pub width: i64,
    /// The height term.
    pub height: i64,
}

impl CropAspect {
    /// `width`:`height`, each at least 1.
    pub fn new(width: i64, height: i64) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
        }
    }

    /// "4x5": safe in a file name and as a dictionary key.
    pub fn key(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    /// "4:5".
    pub fn title(&self) -> String {
        format!("{}:{}", self.width, self.height)
    }

    /// Width over height.
    pub fn ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }

    /// The shapes offered for an output.
    pub const COMMON: [Self; 8] = [
        Self { width: 1, height: 1 },
        Self { width: 4, height: 5 },
        Self { width: 3, height: 4 },
        Self { width: 2, height: 3 },
        Self { width: 9, height: 16 },
        Self { width: 16, height: 9 },
        Self { width: 3, height: 2 },
        Self { width: 4, height: 3 },
    ];
}

/// Where an item's crop sits for one aspect, independent of the picture's pixel size: how far it is zoomed in from
/// the largest frame of that shape, and its centre as a fraction of the picture's width and height. The default is
/// the largest centred frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CropFrame {
    /// 1 is the largest frame of the shape; 2 is half its width and height.
    pub zoom: f64,
    /// 0…1 from the left.
    pub center_x: f64,
    /// 0…1 from the top.
    pub center_y: f64,
}

impl Default for CropFrame {
    fn default() -> Self {
        Self::CENTERED
    }
}

impl CropFrame {
    /// The largest centred frame.
    pub const CENTERED: Self = Self {
        zoom: 1.0,
        center_x: 0.5,
        center_y: 0.5,
    };
    /// The furthest a crop zooms in.
    pub const MAXIMUM_ZOOM: f64 = 4.0;

    /// The crop in a picture of `size`, upright, origin top left: never outside it and at least one pixel.
    pub fn rect(&self, aspect: CropAspect, size: PixelSize) -> PixelRect {
        if !size.is_valid() {
            return PixelRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
        }
        let picture_width = size.width as f64;
        let picture_height = size.height as f64;
        let mut width = picture_width;
        let mut height = picture_height;
        if picture_width / picture_height > aspect.ratio() {
            width = picture_height * aspect.ratio();
        } else {
            height = picture_width / aspect.ratio();
        }
        let zoom = if self.zoom.is_finite() { self.zoom } else { 1.0 };
        let scale = zoom.clamp(1.0, Self::MAXIMUM_ZOOM);
        width = (width / scale).round().max(1.0);
        height = (height / scale).round().max(1.0);
        let x = clamp(self.center_x * picture_width - width / 2.0, picture_width - width);
        let y = clamp(
            self.center_y * picture_height - height / 2.0,
            picture_height - height,
        );
        PixelRect {
            x: x.round() as i64,
            y: y.round() as i64,
            width: width as i64,
            height: height as i64,
        }
    }
}

fn clamp(value: f64, limit: f64) -> f64 {
    if !value.is_finite() {
        return (limit / 2.0).max(0.0);
    }
    value.max(0.0).min(limit.max(0.0))
}

/// A rectangle of whole pixels, origin top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PixelRect {
    /// From the left.
    pub x: i64,
    /// From the top.
    pub y: i64,
    /// Pixels across.
    pub width: i64,
    /// Pixels down.
    pub height: i64,
}

impl PixelRect {
    /// Its size.
    pub fn size(&self) -> PixelSize {
        PixelSize::new(self.width, self.height)
    }
}
