//! The sheet of an Enhance run on this computer (`OnDeviceEnhanceSheet.swift`): its options, how long it takes, that
//! it is free, and before an engine's first download the agreement to it.

use jutsu_core::{
    OnDeviceAction, OnDeviceEngine, OnDeviceEstimate, OnDeviceUpscale, OnDeviceUpscaleTarget, PixelSize,
    SmoothMotion,
};
use serde::{Deserialize, Serialize};

use super::text;

/// Which run the sheet starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OnDeviceKind {
    SmoothMotion,
    UpscaleVideo,
    UpscaleImage,
}

impl OnDeviceKind {
    /// The run, with `motion` for Smooth Motion; Upscale is always 4×.
    pub fn action(self, motion: SmoothMotion) -> OnDeviceAction {
        match self {
            Self::SmoothMotion => OnDeviceAction::SmoothMotion(motion),
            Self::UpscaleVideo => OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full),
            Self::UpscaleImage => OnDeviceAction::UpscaleImage(OnDeviceUpscaleTarget::Full),
        }
    }

    pub fn engine(self) -> OnDeviceEngine {
        self.action(SmoothMotion::Fps60).engine()
    }
}

/// What was read of the file: a video's frame rate, size and length, or an image's size and alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnDeviceFile {
    Video {
        duration: f64,
        frame_rate: f64,
        pixel_size: PixelSize,
    },
    Image {
        pixel_size: PixelSize,
        has_alpha: bool,
    },
    Unreadable,
}

/// What the sheet is built from.
#[derive(Debug, Clone)]
pub struct OnDeviceSheetInput {
    pub kind: OnDeviceKind,
    /// The item's title, under the sheet's.
    pub item_title: String,
    pub file: OnDeviceFile,
    /// The motion asked for; the first offered one when it isn't offered for this clip.
    pub motion: Option<SmoothMotion>,
    /// Why this computer can't run the engine.
    pub unavailable_reason: Option<String>,
    pub needs_consent: bool,
    pub download_size: Option<i64>,
}

/// One motion of the Motion picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionOption {
    /// `SmoothMotion`'s raw value: "fps60", "slowMotion2x", "slowMotion4x".
    pub value: String,
    pub title: String,
}

/// A line of the sheet, muted or a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SheetNote {
    pub text: String,
    pub is_warning: bool,
}

impl SheetNote {
    fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_warning: false,
        }
    }

    fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_warning: true,
        }
    }
}

/// The sheet as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OnDeviceSheet {
    pub kind: OnDeviceKind,
    pub title: String,
    pub item_title: String,
    /// Smooth Motion's choices; empty for Upscale.
    pub motions: Vec<MotionOption>,
    /// The chosen motion's raw value; `None` for Upscale.
    pub motion: Option<String>,
    /// "Size" for Upscale: "4× · 854×480 → 3416×1920".
    pub sizes: Option<String>,
    /// Under the options: the sound, the size limit, the transparency.
    pub option_notes: Vec<SheetNote>,
    /// Before the engine's first download: what is downloaded, from where and how large; `None` once agreed.
    pub model_notice: Option<String>,
    pub consent_title: String,
    /// Free, how long it takes or that the file can't be read, and the new version.
    pub notes: Vec<SheetNote>,
    /// Why this computer can't run it.
    pub unavailable_reason: Option<String>,
    /// The start button's title: "Smooth Motion to 60 fps", "Upscale 4×".
    pub start_title: String,
    pub price_tag: String,
    /// Whether it can start once the download is agreed to, when `model_notice` asks for that.
    pub can_start: bool,
}

impl OnDeviceSheet {
    pub fn new(input: &OnDeviceSheetInput) -> Self {
        let (frame_rate, size) = match input.file {
            OnDeviceFile::Video {
                frame_rate,
                pixel_size,
                ..
            } => (frame_rate, Some(pixel_size)),
            OnDeviceFile::Image { pixel_size, .. } => (0.0, Some(pixel_size)),
            OnDeviceFile::Unreadable => (0.0, None),
        };
        let offered = SmoothMotion::offered(frame_rate);
        let motion = input
            .motion
            .filter(|motion| offered.contains(motion))
            .or_else(|| offered.first().copied())
            .unwrap_or(SmoothMotion::Fps60);
        let action = input.kind.action(motion);
        let is_too_large = size.is_some_and(|size| match input.kind {
            OnDeviceKind::SmoothMotion => false,
            OnDeviceKind::UpscaleVideo => !OnDeviceUpscale::offers_video(size),
            OnDeviceKind::UpscaleImage => !OnDeviceUpscale::offers_image(size),
        });
        let mut option_notes = Vec::new();
        let mut sizes = None;
        match input.kind {
            OnDeviceKind::SmoothMotion => option_notes.push(SheetNote::muted(if motion.keeps_audio() {
                text::SMOOTH_MOTION_NOTE
            } else {
                text::SLOW_MOTION_SOUND
            })),
            OnDeviceKind::UpscaleVideo | OnDeviceKind::UpscaleImage => {
                sizes = size.map(text::upscale_sizes);
                let note = match (input.kind, is_too_large) {
                    (OnDeviceKind::UpscaleImage, _) => text::TOO_LARGE_IMAGE,
                    (_, true) => text::TOO_LARGE_VIDEO,
                    _ => text::UPSCALE_SOUND,
                };
                if is_too_large {
                    option_notes.push(SheetNote::warning(note));
                } else if input.kind == OnDeviceKind::UpscaleVideo {
                    option_notes.push(SheetNote::muted(note));
                }
                if matches!(input.file, OnDeviceFile::Image { has_alpha: true, .. }) && !is_too_large {
                    option_notes.push(SheetNote::warning(text::OPAQUE_RESULT));
                }
            }
        }
        let mut notes = vec![SheetNote::muted(text::PRICE)];
        if let Some(seconds) = expected_time(input.kind, input.file) {
            notes.push(SheetNote::muted(text::expected_time(seconds)));
        } else if input.file == OnDeviceFile::Unreadable {
            notes.push(SheetNote::warning(text::UNREADABLE));
        }
        notes.push(SheetNote::muted(text::VERSION_NOTE));
        Self {
            kind: input.kind,
            title: match input.kind {
                OnDeviceKind::SmoothMotion => text::SMOOTH_MOTION_TITLE,
                OnDeviceKind::UpscaleVideo | OnDeviceKind::UpscaleImage => text::UPSCALE_TITLE,
            }
            .to_owned(),
            item_title: input.item_title.clone(),
            motions: if input.kind == OnDeviceKind::SmoothMotion {
                offered
                    .iter()
                    .map(|motion| MotionOption {
                        value: motion.raw_value().to_owned(),
                        title: text::motion_title(*motion).to_owned(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            motion: (input.kind == OnDeviceKind::SmoothMotion).then(|| motion.raw_value().to_owned()),
            sizes,
            option_notes,
            model_notice: input
                .needs_consent
                .then(|| text::model_notice(input.download_size)),
            consent_title: text::MODEL_CONSENT.to_owned(),
            notes,
            unavailable_reason: input.unavailable_reason.clone(),
            start_title: text::action_title(&action),
            price_tag: text::FREE.to_owned(),
            can_start: input.unavailable_reason.is_none() && size.is_some() && !is_too_large,
        }
    }
}

fn expected_time(kind: OnDeviceKind, file: OnDeviceFile) -> Option<f64> {
    match (kind, file) {
        (
            OnDeviceKind::SmoothMotion,
            OnDeviceFile::Video {
                duration,
                frame_rate,
                pixel_size,
            },
        ) => Some(OnDeviceEstimate::smooth_motion(duration, frame_rate, pixel_size)),
        (
            OnDeviceKind::UpscaleVideo,
            OnDeviceFile::Video {
                duration,
                frame_rate,
                pixel_size,
            },
        ) => Some(OnDeviceEstimate::upscale_video(duration, frame_rate, pixel_size)),
        (OnDeviceKind::UpscaleImage, OnDeviceFile::Image { pixel_size, .. }) => {
            Some(OnDeviceEstimate::upscale_image(pixel_size))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: OnDeviceKind, file: OnDeviceFile) -> OnDeviceSheetInput {
        OnDeviceSheetInput {
            kind,
            item_title: "rooftop".to_owned(),
            file,
            motion: None,
            unavailable_reason: None,
            needs_consent: false,
            download_size: None,
        }
    }

    fn video(width: i64, height: i64, frame_rate: f64) -> OnDeviceFile {
        OnDeviceFile::Video {
            duration: 5.0,
            frame_rate,
            pixel_size: PixelSize::new(width, height),
        }
    }

    #[test]
    fn smooth_motion_offers_the_motions_of_the_clip_and_says_slow_motion_is_silent() {
        let mut asked = input(OnDeviceKind::SmoothMotion, video(854, 480, 24.0));
        let sheet = OnDeviceSheet::new(&asked);
        assert_eq!(sheet.title, "Smooth Motion");
        assert_eq!(sheet.motion.as_deref(), Some("fps60"));
        assert_eq!(sheet.start_title, "Smooth Motion to 60 fps");
        assert_eq!(
            sheet.option_notes,
            vec![SheetNote::muted(text::SMOOTH_MOTION_NOTE)]
        );
        assert!(sheet.can_start);
        assert!(sheet.notes.iter().any(|note| note.text.starts_with("Takes ~")));
        asked.motion = Some(SmoothMotion::SlowMotion2x);
        let slow = OnDeviceSheet::new(&asked);
        assert_eq!(slow.start_title, "2× Slow Motion");
        assert_eq!(slow.option_notes, vec![SheetNote::muted(text::SLOW_MOTION_SOUND)]);
    }

    #[test]
    fn a_motion_not_offered_for_the_clip_falls_back_to_the_first_offered() {
        let mut asked = input(OnDeviceKind::SmoothMotion, video(854, 480, 60.0));
        asked.motion = Some(SmoothMotion::Fps60);
        let sheet = OnDeviceSheet::new(&asked);
        let offered = SmoothMotion::offered(60.0);
        assert!(!offered.contains(&SmoothMotion::Fps60));
        assert_eq!(
            sheet.motion.as_deref(),
            offered.first().map(|motion| motion.raw_value())
        );
        assert_eq!(sheet.motions.len(), offered.len());
    }

    #[test]
    fn upscale_shows_the_sizes_and_refuses_a_video_over_720p() {
        let sheet = OnDeviceSheet::new(&input(OnDeviceKind::UpscaleVideo, video(854, 480, 24.0)));
        assert_eq!(sheet.title, "Upscale on This Computer");
        assert_eq!(sheet.sizes.as_deref(), Some("4× · 854×480 → 3416×1920"));
        assert_eq!(sheet.option_notes, vec![SheetNote::muted(text::UPSCALE_SOUND)]);
        assert!(sheet.can_start);
        let large = OnDeviceSheet::new(&input(OnDeviceKind::UpscaleVideo, video(1920, 1080, 24.0)));
        assert_eq!(
            large.option_notes,
            vec![SheetNote::warning(text::TOO_LARGE_VIDEO)]
        );
        assert!(!large.can_start);
    }

    #[test]
    fn an_image_with_transparency_is_warned_it_comes_out_opaque() {
        let file = OnDeviceFile::Image {
            pixel_size: PixelSize::new(512, 512),
            has_alpha: true,
        };
        let sheet = OnDeviceSheet::new(&input(OnDeviceKind::UpscaleImage, file));
        assert_eq!(sheet.option_notes, vec![SheetNote::warning(text::OPAQUE_RESULT)]);
        assert_eq!(sheet.start_title, "Upscale 4×");
        assert!(sheet.can_start);
    }

    #[test]
    fn an_unreadable_file_or_a_missing_engine_cannot_start() {
        let unreadable = OnDeviceSheet::new(&input(OnDeviceKind::UpscaleVideo, OnDeviceFile::Unreadable));
        assert!(!unreadable.can_start);
        assert!(unreadable.notes.contains(&SheetNote::warning(text::UNREADABLE)));
        let mut missing = input(OnDeviceKind::SmoothMotion, video(854, 480, 24.0));
        missing.unavailable_reason = Some("ffmpeg isn't installed.".to_owned());
        assert!(!OnDeviceSheet::new(&missing).can_start);
    }

    #[test]
    fn the_first_upscale_asks_to_download_the_engine_with_its_size() {
        let mut asked = input(OnDeviceKind::UpscaleVideo, video(854, 480, 24.0));
        asked.needs_consent = true;
        asked.download_size = Some(51_817_124);
        let sheet = OnDeviceSheet::new(&asked);
        let notice = sheet.model_notice.unwrap_or_default();
        assert!(notice.contains("Real-ESRGAN"), "{notice}");
        assert!(notice.contains("GitHub"), "{notice}");
        assert!(notice.contains("MB"), "{notice}");
        assert_eq!(sheet.consent_title, "Download the model");
    }
}
