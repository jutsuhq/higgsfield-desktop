//! The texts of Enhance's runs on this computer (`OnDeviceContent.swift`), with "this Mac" said as "this computer",
//! and which runs a result's page offers (`OnDeviceOffer`).

use jutsu_core::{
    OnDeviceAction, OnDeviceEngine, OnDeviceUpscale, OnDeviceUpscaleTarget, PixelSize, SmoothMotion,
};

use super::runs::{OnDeviceRun, OnDeviceRunState};
use crate::library::item::LibraryItem;
use crate::queue::format;

pub const PRICE: &str = "Free · runs on this computer";
pub const VERSION_NOTE: &str =
    "The result is added as a new version of this item; the version it is made from stays as it is.";
pub const SMOOTH_MOTION_NOTE: &str =
    "New frames are made between the clip's own. Across a cut a frame is repeated instead.";
pub const SLOW_MOTION_SOUND: &str = "Slow motion plays without sound.";
pub const WAITING: &str = "Waiting for the run before it";
pub const DOWNLOADING_MODEL: &str = "Downloading the model";
pub const SMOOTH_MOTION_TITLE: &str = "Smooth Motion";
pub const UPSCALE_TITLE: &str = "Upscale on This Computer";
pub const UPSCALE_SOUND: &str = "The sound is kept as it is.";
pub const MODEL_CONSENT: &str = "Download the model";
pub const TOO_LARGE_VIDEO: &str = "Upscale on this computer takes videos up to 720p.";
pub const OPAQUE_RESULT: &str = "This image has transparency; the upscaled image comes out opaque.";
pub const TOO_LARGE_IMAGE: &str = "Upscale on this computer takes images up to 1920×1080.";
pub const UNREADABLE: &str = "This file can't be read.";
pub const FREE: &str = "Free";

/// Before the first run of an engine with a model: here Real-ESRGAN, downloaded from its GitHub release rather than
/// by the system, so the notice names it and its size.
pub fn model_notice(download_size: Option<i64>) -> String {
    let size = download_size.map_or_else(String::new, |bytes| {
        format!(", about {}", jutsu_media::byte_count_string(bytes))
    });
    format!(
        "Upscaling on this computer needs Real-ESRGAN, which Jutsu Studio downloads from its GitHub release once{size}. It is downloaded when the first upscale starts, and stays for later ones."
    )
}

/// "4× · 854×480 → 3416×1920".
pub fn upscale_sizes(source: PixelSize) -> String {
    let scale = OnDeviceUpscale::SCALE;
    format!(
        "{scale}× · {}×{} → {}×{}",
        source.width,
        source.height,
        source.width * scale,
        source.height * scale
    )
}

/// "Takes ~2 min on this computer".
pub fn expected_time(seconds: f64) -> String {
    format!("Takes {} on this computer", format::approximate(seconds))
}

/// "Smooth Motion to 60 fps · 42%", or that it waits.
pub fn progress(run: &OnDeviceRun) -> String {
    let title = action_title(&run.request.action);
    match run.state {
        OnDeviceRunState::Waiting => format!("{title} · {WAITING}"),
        OnDeviceRunState::DownloadingModel(fraction) => {
            format!("{DOWNLOADING_MODEL} · {}", format::percent(fraction))
        }
        OnDeviceRunState::Working(fraction) => format!("{title} · {}", format::percent(fraction)),
    }
}

/// Why Enhance's runs on this computer are off for an item while one waits or runs on it: "Wait until Upscale 4×
/// finishes".
pub fn busy(run: &OnDeviceRun) -> String {
    format!("Wait until {} finishes", action_title(&run.request.action))
}

/// The Enhance menu's hint when only this computer's runs are on offer.
pub fn menu_hint(offer: OnDeviceOffer) -> &'static str {
    if offer.upscale_video {
        "Smooth Motion and Upscale, free on this computer"
    } else {
        "Smooth Motion, free on this computer"
    }
}

pub fn failure_title(action: &OnDeviceAction) -> String {
    format!("Can't finish {}", action_title(action))
}

pub fn motion_title(motion: SmoothMotion) -> &'static str {
    match motion {
        SmoothMotion::Fps60 => "60 fps",
        SmoothMotion::SlowMotion2x => "2× Slow Motion",
        SmoothMotion::SlowMotion4x => "4× Slow Motion",
    }
}

/// "Smooth Motion to 60 fps", "2× Slow Motion", "Upscale 4×", "Upscale to 1080p".
pub fn action_title(action: &OnDeviceAction) -> String {
    match action {
        OnDeviceAction::SmoothMotion(SmoothMotion::Fps60) => "Smooth Motion to 60 fps".to_owned(),
        OnDeviceAction::SmoothMotion(motion) => motion_title(*motion).to_owned(),
        OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full)
        | OnDeviceAction::UpscaleImage(OnDeviceUpscaleTarget::Full) => "Upscale 4×".to_owned(),
        OnDeviceAction::UpscaleVideo(target) | OnDeviceAction::UpscaleImage(target) => {
            format!("Upscale to {}", target.file_suffix())
        }
    }
}

/// Which of Enhance's runs on this computer a result's page offers: only for a result with its file here, and only
/// those this computer's engines can run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OnDeviceOffer {
    pub smooth_motion: bool,
    /// Only for a result up to 720p (`OnDeviceUpscale::offers_video`).
    pub upscale_video: bool,
}

impl OnDeviceOffer {
    pub const NONE: Self = Self {
        smooth_motion: false,
        upscale_video: false,
    };

    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }

    /// `supports`: whether this computer has the engine (`OnDeviceRuns::supports`).
    pub fn of(item: &LibraryItem, supports: impl Fn(OnDeviceEngine) -> bool) -> Self {
        if item.output_file().is_none() || item.needs_import() {
            return Self::NONE;
        }
        Self {
            smooth_motion: supports(OnDeviceEngine::FrameRateConversion),
            upscale_video: supports(OnDeviceEngine::VideoSuperResolution)
                && item
                    .job
                    .output_pixel_size()
                    .is_some_and(OnDeviceUpscale::offers_video),
        }
    }

    /// Whether a saved image of `size` (upright, `None` when unread) is offered Upscale › On this computer.
    pub fn upscales_image(size: Option<PixelSize>, supports: impl Fn(OnDeviceEngine) -> bool) -> bool {
        supports(OnDeviceEngine::ImageSuperResolution) && size.is_some_and(OnDeviceUpscale::offers_image)
    }
}
