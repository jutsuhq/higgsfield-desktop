//! Card frames (`ThumbnailFit.swift`, `LibraryThumbnail`): how a frame sits in the grid's 16:9 tile, and the cached
//! frame of a video taken with ffmpeg.

use std::path::{Path, PathBuf};

use jutsu_media::{FfmpegTools, frames};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// How a frame sits in a tile of a fixed shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailFit {
    /// Fills the tile; the little it crops isn't worth bars.
    Fill,
    /// Shown whole, over a blurred copy of itself filling the rest of the tile.
    FitOverBackdrop,
}

impl ThumbnailFit {
    /// The widest a frame may be, or the narrowest, relative to the tile and still fill it.
    const FILL_TOLERANCE: f64 = 1.12;

    /// A frame close to the tile's shape fills it; a portrait, square, 4:3 or ultra-wide frame is fitted whole. A
    /// frame without a usable size fills.
    pub fn new(width_over_height: Option<f64>, tile_aspect: f64) -> Self {
        let Some(ratio) = width_over_height.filter(|ratio| ratio.is_finite() && *ratio > 0.0) else {
            return Self::Fill;
        };
        if tile_aspect <= 0.0 {
            return Self::Fill;
        }
        let relative = ratio / tile_aspect;
        if (1.0 / Self::FILL_TOLERANCE..=Self::FILL_TOLERANCE).contains(&relative) {
            Self::Fill
        } else {
            Self::FitOverBackdrop
        }
    }
}

/// The grid's tile shape.
pub const TILE_ASPECT: f64 = 16.0 / 9.0;

/// Where the frame of a video's version is cached.
pub fn cached_frame(cache: &Path, version: Uuid) -> PathBuf {
    cache.join("thumbnails").join(format!("{version}.png"))
}

/// Writes the frame from the middle of `video` to `target`, unless it is there already.
pub async fn write_frame(tools: &FfmpegTools, video: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Ok(());
    }
    let images = frames::thumbnail_strip(tools, video, 1, &CancellationToken::new())
        .await
        .map_err(|error| error.to_string())?;
    let image = images
        .first()
        .ok_or_else(|| "The video has no frame.".to_owned())?;
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    frames::write_png(image, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{TILE_ASPECT, ThumbnailFit};

    fn fit(width: i32, height: i32) -> ThumbnailFit {
        let ratio = (height != 0).then(|| f64::from(width) / f64::from(height));
        ThumbnailFit::new(ratio, TILE_ASPECT)
    }

    #[test]
    fn frames_close_to_the_tile_fill_it() {
        for (width, height) in [(1920, 1080), (1280, 720), (1920, 1200), (854, 480)] {
            assert_eq!(fit(width, height), ThumbnailFit::Fill, "{width}×{height}");
        }
    }

    #[test]
    fn other_shapes_are_fitted_whole() {
        for (width, height) in [
            (1080, 1920),
            (720, 1280),
            (1080, 1080),
            (1440, 1080),
            (2560, 1080),
        ] {
            assert_eq!(
                fit(width, height),
                ThumbnailFit::FitOverBackdrop,
                "{width}×{height}"
            );
        }
    }

    #[test]
    fn frames_without_a_size_fill() {
        for (width, height) in [(0, 0), (1920, 0), (0, 1080)] {
            assert_eq!(fit(width, height), ThumbnailFit::Fill, "{width}×{height}");
        }
    }
}
