//! Genjutsu's galleries in the app's terms: styles become `StylePreset`s keyed by the UUID in their ID, motion
//! entries become `MotionPreset`s.

use higgsfield::mcp::account::{GenjutsuCategory, GenjutsuPreset, VideoRequest};
use jutsu_core::{Mode, MotionPreset, MotionShelf, StylePreset};
use uuid::Uuid;

pub(crate) const STYLE_PREFIX: &str = "genjutsu-style:";

/// The ID `execute_preset` takes for the style `JobSpec::preset_id` names.
pub(crate) fn style_id(preset_id: Uuid) -> String {
    format!("{STYLE_PREFIX}{}", preset_id.hyphenated())
}

/// `None` for anything but a style with a still preview.
pub(crate) fn style(preset: &GenjutsuPreset) -> Option<StylePreset> {
    if !preset.id.starts_with(STYLE_PREFIX) {
        return None;
    }
    let id = preset.uuid()?;
    let still = preset.thumbnail_url.clone()?;
    Some(StylePreset {
        id,
        name: preset.name.clone(),
        preview_url: still,
        preview_video_url: preset.preview_url.clone(),
    })
}

/// `None` for an entry without a driving video, which can't be a source.
pub(crate) fn motion(preset: &GenjutsuPreset) -> Option<MotionPreset> {
    let video = preset.driving_video_url()?;
    let mode = match preset.example_model() {
        Some(VideoRequest::MOTION_CONTROL) => Some(Mode::MotionTransfer),
        Some(VideoRequest::REPLACE_OBJECT) => Some(Mode::ObjectSwap),
        _ => None,
    };
    Some(MotionPreset {
        id: preset.id.clone(),
        name: preset.name.clone(),
        video_url: video,
        thumbnail_url: preset.thumbnail_url.clone(),
        preview_url: preset.preview_url.clone(),
        example_mode: mode,
    })
}

/// `None` for the account's uploads, which aren't a gallery.
pub(crate) fn category(shelf: MotionShelf) -> Option<GenjutsuCategory> {
    match shelf {
        MotionShelf::Featured => Some(GenjutsuCategory::Higgsfield),
        MotionShelf::Trending => Some(GenjutsuCategory::Trending),
        MotionShelf::New => Some(GenjutsuCategory::New),
        MotionShelf::Uploads => None,
    }
}
