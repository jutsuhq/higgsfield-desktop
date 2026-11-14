//! Where a restored window may stay. A saved position can point at a display that is gone or arranged differently,
//! and no platform reliably moves such a window back on screen.

/// A rectangle in physical pixels on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// How much of the title bar must be on a display for the person to grab it.
const GRAB_WIDTH: i64 = 100;
const GRAB_HEIGHT: i64 = 30;

/// Whether enough of the window's title bar is on one of `screens` to drag the window.
pub fn can_be_grabbed(window: Frame, screens: &[Frame]) -> bool {
    let bar = (
        i64::from(window.x),
        i64::from(window.y),
        i64::from(window.x) + i64::from(window.width),
        i64::from(window.y) + GRAB_HEIGHT,
    );
    screens.iter().any(|screen| {
        let left = bar.0.max(i64::from(screen.x));
        let top = bar.1.max(i64::from(screen.y));
        let right = bar.2.min(i64::from(screen.x) + i64::from(screen.width));
        let bottom = bar.3.min(i64::from(screen.y) + i64::from(screen.height));
        right - left >= GRAB_WIDTH && bottom - top >= GRAB_HEIGHT
    })
}

#[cfg(test)]
mod tests {
    use super::{Frame, can_be_grabbed};

    const fn frame(x: i32, y: i32, width: u32, height: u32) -> Frame {
        Frame { x, y, width, height }
    }

    /// A window saved on a display that is now gone, or below the only one, is moved back.
    #[test]
    fn only_a_title_bar_on_a_display_counts() {
        let laptop = frame(0, 0, 2940, 1912);
        let external = frame(2940, 0, 3840, 2160);
        assert!(can_be_grabbed(frame(100, 100, 1920, 1200), &[laptop]));
        assert!(can_be_grabbed(frame(3000, 100, 1920, 1200), &[laptop, external]));
        // The bar hangs off the bottom edge: the body is visible but nothing to drag.
        assert!(!can_be_grabbed(frame(2860, 1848, 2940, 1844), &[laptop]));
        // Saved on the external display, which is unplugged now.
        assert!(!can_be_grabbed(frame(3000, 100, 1920, 1200), &[laptop]));
        // Just a sliver at the right edge.
        assert!(!can_be_grabbed(frame(2900, 100, 1920, 1200), &[laptop]));
        assert!(!can_be_grabbed(frame(0, 0, 800, 600), &[]));
    }
}
