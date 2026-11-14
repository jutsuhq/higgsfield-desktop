//! The pictures the app draws for the system: the tray icon with the work's progress under it (`MenuBarIcon.swift`)
//! and the taskbar overlay that stands for the Dock badge where the taskbar shows no count. Plain RGBA, so `src-tauri`
//! hands them to the platform as they are.

use super::tray::ICON_STEPS;

/// The lime of the controls (`--color-accent`).
pub const ACCENT: [u8; 4] = [0xd4, 0xff, 0x2a, 0xff];
/// The track under the progress.
const TRACK: [u8; 4] = [0x1c, 0x1c, 0x20, 0xff];

/// A screen rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Where the tray panel's top-left corner goes: centred on the icon, under it when the icon sits in the top half of
/// the work area (a top bar), above it otherwise (a taskbar at the bottom), and always inside the work area.
pub fn panel_origin(icon: ScreenRect, panel: (i32, i32), area: ScreenRect) -> (i32, i32) {
    const GAP: i32 = 6;
    let x = icon.x + icon.width / 2 - panel.0 / 2;
    let below = icon.y + icon.height / 2 < area.y + area.height / 2;
    let y = if below {
        icon.y + icon.height + GAP
    } else {
        icon.y - panel.1 - GAP
    };
    let inside = |value: i32, low: i32, high: i32| value.min(high).max(low);
    (
        inside(x, area.x, area.x + area.width - panel.0),
        inside(y, area.y, area.y + area.height - panel.1),
    )
}

/// The icon with a progress bar along its bottom: `fraction` rounded down to one of `ICON_STEPS` steps, so the icon
/// changes a dozen times over a job. `rgba` must hold `width × height` pixels; anything else comes back unchanged.
pub fn with_progress(rgba: &[u8], width: u32, height: u32, fraction: f64) -> Vec<u8> {
    let mut pixels = rgba.to_vec();
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || pixels.len() != w * h * 4 {
        return pixels;
    }
    let bar = (h / 6).max(2);
    let steps = f64::from(ICON_STEPS);
    let step = (fraction.clamp(0.0, 1.0) * steps).floor() / steps;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a fraction of the icon's width"
    )]
    let filled = (step * w as f64).round() as usize;
    for row in h - bar..h {
        for column in 0..w {
            let color = if column < filled { ACCENT } else { TRACK };
            let start = (row * w + column) * 4;
            pixels[start..start + 4].copy_from_slice(&color);
        }
    }
    pixels
}

/// A lime dot on transparency, `size` pixels square: Windows' taskbar overlay, which shows a picture and no number.
pub fn badge_dot(diameter: u32) -> Vec<u8> {
    let side = diameter as usize;
    let mut pixels = vec![0; side * side * 4];
    #[allow(clippy::cast_precision_loss, reason = "a small icon")]
    let (center, radius) = (side as f64 / 2.0, side as f64 / 2.0 - 0.5);
    for row in 0..side {
        for column in 0..side {
            #[allow(clippy::cast_precision_loss, reason = "a small icon")]
            let (dx, dy) = (column as f64 + 0.5 - center, row as f64 + 0.5 - center);
            if dx.hypot(dy) <= radius {
                let start = (row * side + column) * 4;
                pixels[start..start + 4].copy_from_slice(&ACCENT);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], width: u32, x: usize, y: usize) -> [u8; 4] {
        let start = (y * width as usize + x) * 4;
        [rgba[start], rgba[start + 1], rgba[start + 2], rgba[start + 3]]
    }

    #[test]
    fn the_bar_fills_by_steps_along_the_bottom() {
        let blank = vec![0; 24 * 24 * 4];
        let half = with_progress(&blank, 24, 24, 0.5);
        assert_eq!(pixel(&half, 24, 0, 23), ACCENT);
        assert_eq!(pixel(&half, 24, 11, 23), ACCENT);
        assert_eq!(pixel(&half, 24, 12, 23), TRACK);
        assert_eq!(pixel(&half, 24, 0, 0), [0; 4], "the icon above the bar stays");
        // 0.54 is still the sixth of twelve steps.
        assert_eq!(with_progress(&blank, 24, 24, 0.54), half);
        let none = with_progress(&blank, 24, 24, 0.0);
        assert_eq!(pixel(&none, 24, 0, 23), TRACK);
        let full = with_progress(&blank, 24, 24, 3.0);
        assert_eq!(pixel(&full, 24, 23, 23), ACCENT);
    }

    #[test]
    fn a_picture_of_the_wrong_size_comes_back_as_it_was() {
        assert_eq!(with_progress(&[1, 2, 3], 24, 24, 0.5), vec![1, 2, 3]);
    }

    #[test]
    fn the_dot_is_round() {
        let dot = badge_dot(16);
        assert_eq!(dot.len(), 16 * 16 * 4);
        assert_eq!(pixel(&dot, 16, 8, 8), ACCENT);
        assert_eq!(pixel(&dot, 16, 0, 0), [0; 4]);
    }

    #[test]
    fn the_panel_opens_by_the_icon_inside_the_screen() {
        let screen = ScreenRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        // A taskbar at the bottom: above the icon.
        let bottom = ScreenRect {
            x: 1700,
            y: 1045,
            width: 24,
            height: 30,
        };
        assert_eq!(panel_origin(bottom, (360, 520), screen), (1532, 1045 - 520 - 6));
        // A top bar: under the icon.
        let top = ScreenRect {
            x: 900,
            y: 0,
            width: 24,
            height: 26,
        };
        assert_eq!(panel_origin(top, (360, 520), screen), (732, 32));
        // Near the right edge: kept on screen.
        let corner = ScreenRect { x: 1900, ..top };
        assert_eq!(panel_origin(corner, (360, 520), screen).0, 1560);
    }
}
