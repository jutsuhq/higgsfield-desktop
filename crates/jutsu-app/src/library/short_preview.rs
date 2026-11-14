//! "Export Short Preview" (`ShortPreview.swift`): the start of a result, small enough for a messenger to show
//! inline.

use std::path::Path;

use jutsu_core::{ClosedRange, PixelSize};

/// Seconds the preview keeps.
pub const LENGTH: f64 = 5.0;
/// 480p: the shorter side, in pixels.
pub const SHORT_SIDE: i64 = 480;
/// Decimal megabytes, as messengers count them.
pub const BYTE_LIMIT: i64 = 2_000_000;
/// The menu item.
pub const MENU_TITLE: &str = "Export Short Preview…";
/// The menu item's tooltip.
pub const HELP: &str = "The first 5 seconds at 480p without sound, small enough to send in a messenger";
/// The alert's title when the export fails.
pub const FAILED: &str = "Can't export the preview";

/// The first `LENGTH` seconds, or the whole clip when it is shorter; `None` for a clip with no length.
pub fn time_range(duration: f64) -> Option<ClosedRange<f64>> {
    (duration.is_finite() && duration > 0.0).then(|| ClosedRange::new(0.0, LENGTH.min(duration)))
}

/// `source` scaled so its shorter side is 480, with even sides for H.264; a smaller source keeps its size.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn render_size(source: PixelSize) -> Option<PixelSize> {
    if !source.is_valid() {
        return None;
    }
    let scale = (SHORT_SIDE as f64 / source.width.min(source.height) as f64).min(1.0);
    let even = |side: i64| (((side as f64 * scale / 2.0).round() as i64) * 2).max(2);
    Some(PixelSize {
        width: even(source.width),
        height: even(source.height),
    })
}

/// "result-preview.mp4" for "result.mov".
pub fn file_name(output: &Path) -> String {
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Preview");
    format!("{stem}-preview.mp4")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: i64, height: i64) -> PixelSize {
        PixelSize { width, height }
    }

    /// `ShortPreviewTests.takesTheFirstFiveSeconds`, `keepsAShorterClipWhole`, `clipWithoutLengthHasNoRange`.
    #[test]
    fn takes_the_first_five_seconds_or_the_whole_clip() {
        let bounds = |range: Option<ClosedRange<f64>>| range.map(|range| (range.lower(), range.upper()));
        assert_eq!(bounds(time_range(12.4)), Some((0.0, 5.0)));
        assert_eq!(bounds(time_range(3.2)), Some((0.0, 3.2)));
        assert_eq!(bounds(time_range(0.0)), None);
        assert_eq!(bounds(time_range(f64::NAN)), None);
    }

    /// `ShortPreviewTests.scalesTheShorterSideTo480`.
    #[test]
    fn scales_the_shorter_side_to_480() {
        assert_eq!(render_size(size(1920, 1080)), Some(size(854, 480)));
        assert_eq!(render_size(size(1080, 1920)), Some(size(480, 854)));
        assert_eq!(render_size(size(1080, 1080)), Some(size(480, 480)));
    }

    /// `ShortPreviewTests.neverUpscalesAndKeepsSidesEven`.
    #[test]
    fn never_upscales_and_keeps_sides_even() {
        assert_eq!(render_size(size(640, 360)), Some(size(640, 360)));
        let odd = render_size(size(1281, 721)).unwrap();
        assert!(odd.width % 2 == 0 && odd.height % 2 == 0);
        assert_eq!(render_size(size(0, 720)), None);
    }

    /// `ShortPreviewTests.namesThePreviewAfterTheResult`, `fitsUnderTwoMegabytes`.
    #[test]
    fn names_the_preview_after_the_result() {
        assert_eq!(
            file_name(Path::new("/out/2026-10-14_motion_1a2b.mov")),
            "2026-10-14_motion_1a2b-preview.mp4"
        );
    }
}
