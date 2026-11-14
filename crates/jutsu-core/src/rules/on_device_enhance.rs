//! On-device Enhance: what a run makes, the engines it runs on, and its sizes and times.
//!
//! The engines are VideoToolbox's frame processors on macOS; here they are values only, and a platform runs them
//! behind `OnDeviceEnhancing`.

use crate::models::{OutputMedia, PixelSize};
use crate::rules::SmoothMotion;
use crate::support::Seconds;
use crate::support::string_enum;
use crate::support::text::swift_double;

/// What an on-device Enhance run makes from the item it was started on (`OnDeviceEnhancing`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnDeviceAction {
    /// Frames made between the clip's own.
    SmoothMotion(SmoothMotion),
    /// A clip up to 720p at `target`, 4× its frame size or less (`OnDeviceUpscale`).
    UpscaleVideo(OnDeviceUpscaleTarget),
    /// An image up to 1920×1080 at `target`, 4× its size or less (`OnDeviceUpscale::offers_image`).
    UpscaleImage(OnDeviceUpscaleTarget),
}

impl OnDeviceAction {
    /// The engine the action runs on.
    pub fn engine(&self) -> OnDeviceEngine {
        match self {
            Self::SmoothMotion(_) => OnDeviceEngine::FrameRateConversion,
            Self::UpscaleVideo(_) => OnDeviceEngine::VideoSuperResolution,
            Self::UpscaleImage(_) => OnDeviceEngine::ImageSuperResolution,
        }
    }

    /// What the result's file name adds to the source's: "rooftop 60fps.mp4", "rooftop 4x.mp4", "rooftop 1080p.mp4".
    pub fn file_suffix(&self) -> String {
        match self {
            Self::SmoothMotion(SmoothMotion::Fps60) => "60fps".to_owned(),
            Self::SmoothMotion(SmoothMotion::SlowMotion2x) => "slow 2x".to_owned(),
            Self::SmoothMotion(SmoothMotion::SlowMotion4x) => "slow 4x".to_owned(),
            Self::UpscaleVideo(target) | Self::UpscaleImage(target) => target.file_suffix(),
        }
    }

    /// The size an upscale ends at; `None` for another run.
    pub fn upscale_target(&self) -> Option<OnDeviceUpscaleTarget> {
        match self {
            Self::SmoothMotion(_) => None,
            Self::UpscaleVideo(target) | Self::UpscaleImage(target) => Some(*target),
        }
    }
}

string_enum! {
    /// The frame processors on-device Enhance runs on (VideoToolbox's on macOS).
    pub enum OnDeviceEngine {
        /// Makes frames between a clip's own (Smooth Motion).
        FrameRateConversion = "frameRateConversion",
        /// Super resolution of the video kind, which uses the frames before to keep detail steady.
        VideoSuperResolution = "videoSuperResolution",
        /// Super resolution of the image kind, one picture at a time.
        ImageSuperResolution = "imageSuperResolution",
    }
}

impl OnDeviceEngine {
    /// Whether the engine needs a model macOS downloads once (about 62 MB a kind) before its first run. Merely
    /// setting the engine up for a size starts that download, so it is done only once the person has agreed.
    pub fn downloads_model(self) -> bool {
        match self {
            Self::FrameRateConversion => false,
            Self::VideoSuperResolution | Self::ImageSuperResolution => true,
        }
    }
}

/// How the scaler takes frames of one size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UpscalePlan {
    /// The frames as the scaler gets them: the source's, or turned a quarter when only that way they fit.
    pub scaler_input: PixelSize,
    /// Whether the frames are turned.
    pub rotates: bool,
    /// 4× the source, in its own orientation.
    pub output: PixelSize,
}

/// Upscale › On this Mac: 4× the frame size with the platform's super resolution, whose only factor is 4.
pub struct OnDeviceUpscale;

impl OnDeviceUpscale {
    /// The scaler's only factor.
    pub const SCALE: i64 = 4;
    /// The largest frame the scaler takes, landscape: wider or taller frames are refused, portrait 720p included.
    pub const SCALER_LIMIT: PixelSize = PixelSize::new(1920, 1080);
    /// The largest video offered, either way round (`offers_video`).
    pub const VIDEO_LIMIT: PixelSize = PixelSize::new(1280, 720);

    /// Video sources up to 720p either way round: at 1080p the 4× frames ran a 16 GB Mac out of memory (#13).
    pub fn offers_video(size: PixelSize) -> bool {
        size.is_valid()
            && size.width.max(size.height) <= Self::VIDEO_LIMIT.width
            && size.width.min(size.height) <= Self::VIDEO_LIMIT.height
    }

    /// Images up to 1920×1080 either way round. The image kind takes 1920 a side in both orientations, so an image
    /// is never turned; the 1080 side keeps a 4× picture's memory in bounds.
    pub fn offers_image(size: PixelSize) -> bool {
        size.is_valid()
            && size.width.max(size.height) <= Self::SCALER_LIMIT.width
            && size.width.min(size.height) <= Self::SCALER_LIMIT.height
    }

    /// Frames of `size` as they are when they fit `limit` (`SCALER_LIMIT` in Swift's default), else turned a
    /// quarter; `None` when neither way fits.
    pub fn plan(size: PixelSize, limit: PixelSize) -> Option<UpscalePlan> {
        if !size.is_valid() {
            return None;
        }
        let output = PixelSize::new(size.width * Self::SCALE, size.height * Self::SCALE);
        if size.width <= limit.width && size.height <= limit.height {
            return Some(UpscalePlan {
                scaler_input: size,
                rotates: false,
                output,
            });
        }
        if size.height > limit.width || size.width > limit.height {
            return None;
        }
        Some(UpscalePlan {
            scaler_input: PixelSize::new(size.height, size.width),
            rotates: true,
            output,
        })
    }
}

/// The size an upscale on this Mac ends at: the scaler's own 4×, or that scaled down to a delivery size with the
/// source's shape. A 720p portrait clip at 4× is 2880×5120, and agents scaled such results to 1080×1920 with
/// tools of their own (#38).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnDeviceUpscaleTarget {
    /// The scaler's own 4×.
    Full,
    /// The short side this many pixels: 1080 (1080p), 1440 (2k) or 2160 (4k).
    ShortSide(i64),
    /// This many times the source's sides, more than 1 and less than 4.
    Factor(f64),
}

impl OnDeviceUpscaleTarget {
    /// What `from_name` takes, for an agent.
    pub const NAMES: &'static str = "1080p, 2k, 4k, or a factor more than 1 and at most 4 such as 2x or 2.5x";

    /// "1080p", "2k", "4k" or a factor such as "2x" or "2.5x" (more than 1, at most 4); `None` for anything else.
    #[allow(clippy::float_cmp, reason = "exactly 4, as in Swift")]
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        match name.as_str() {
            "1080p" => Some(Self::ShortSide(1080)),
            "2k" => Some(Self::ShortSide(1440)),
            "4k" => Some(Self::ShortSide(2160)),
            factor => {
                let value: f64 = factor.strip_suffix('x')?.parse().ok()?;
                let scale = OnDeviceUpscale::SCALE as f64;
                if !value.is_finite() || value <= 1.0 || value > scale {
                    return None;
                }
                Some(if value == scale {
                    Self::Full
                } else {
                    Self::Factor(value)
                })
            }
        }
    }

    /// What the result's file name adds: "4x", "1080p", "2k", "4k", "2.5x".
    #[allow(clippy::float_cmp, reason = "a whole factor, as in Swift")]
    pub fn file_suffix(&self) -> String {
        match self {
            Self::Full => format!("{}x", OnDeviceUpscale::SCALE),
            Self::ShortSide(1440) => "2k".to_owned(),
            Self::ShortSide(2160) => "4k".to_owned(),
            Self::ShortSide(side) => format!("{side}p"),
            Self::Factor(value) if *value == value.round() => format!("{}x", *value as i64),
            Self::Factor(value) => format!("{}x", swift_double(*value)),
        }
    }

    /// The result's size for frames of `source`, in their orientation: the source's shape with even sides, the
    /// short side exactly the one asked for. `None` when that isn't larger than the source or needs more than 4×.
    pub fn output(&self, source: PixelSize) -> Option<PixelSize> {
        if !source.is_valid() {
            return None;
        }
        let scale = OnDeviceUpscale::SCALE;
        let factor = match self {
            Self::Full => return Some(PixelSize::new(source.width * scale, source.height * scale)),
            Self::Factor(value) => *value,
            Self::ShortSide(side) => *side as f64 / source.width.min(source.height) as f64,
        };
        if factor <= 1.0 || factor > scale as f64 {
            return None;
        }
        let even = |side: i64| (((side as f64 * factor / 2.0).round() as i64) * 2).min(side * scale);
        Some(PixelSize::new(even(source.width), even(source.height)))
    }
}

string_enum! {
    /// The video codec of an on-device result: the clip's own where it can hold the frame, else HEVC.
    pub enum EnhanceCodec {
        /// H.264.
        H264 = "h264",
        /// HEVC.
        Hevc = "hevc",
    }
}

impl EnhanceCodec {
    /// H.264 holds frames up to 4096 pixels a side; HEVC takes larger ones and any source that isn't H.264.
    pub fn of(output: PixelSize, source_is_h264: bool) -> Self {
        if source_is_h264 && output.width.max(output.height) <= 4096 {
            Self::H264
        } else {
            Self::Hevc
        }
    }
}

/// How long an on-device run takes, from the benchmark on an Apple silicon Mac (#13), for the sheet to say before it
/// starts. Rough: other work on the Mac slows a run down.
pub struct OnDeviceEstimate;

impl OnDeviceEstimate {
    /// The first run starts the engine, which took up to about 10 seconds.
    const ENGINE_START: Seconds = 10.0;

    /// 50 ms a source frame at 480p, growing with the pixel count to the power 0.6: larger frames took less than
    /// their pixels would say, 80 ms at 720p in the benchmark (#13) and 130 ms at 1080p in the app's own run.
    /// Encoding included.
    pub fn smooth_motion(duration: Seconds, frame_rate: f64, pixel_size: PixelSize) -> Seconds {
        let pixels = (pixel_size.width * pixel_size.height).max(0) as f64;
        let per_frame = 0.05 * (pixels / f64::from(854 * 480)).powf(0.6);
        Self::ENGINE_START + duration.max(0.0) * frame_rate.max(1.0) * per_frame
    }

    /// A straight line through 1 s a frame at 480p and 1.95 s at 720p (portrait, turned), encoding included. A quiet
    /// Mac did a 480p frame in 0.49–0.76 s and one busy with other work in up to 0.96 s, so a run seldom takes longer.
    pub fn upscale_video(duration: Seconds, frame_rate: f64, pixel_size: PixelSize) -> Seconds {
        let per_frame = 0.2389 + 1.8566e-6 * (pixel_size.width * pixel_size.height) as f64;
        Self::ENGINE_START + duration.max(0.0) * frame_rate.max(1.0) * per_frame
    }

    /// One picture, at the video line's cost of a frame.
    pub fn upscale_image(pixel_size: PixelSize) -> Seconds {
        Self::upscale_video(1.0, 1.0, pixel_size)
    }
}

/// How far an on-device run is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnDeviceProgress {
    /// 0…1 of the engine's model, downloaded before its first run.
    DownloadingModel(f64),
    /// 0…1 of the frames.
    Working(f64),
}

/// The file an on-device run wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct OnDeviceOutput {
    /// Upright, as the file shows.
    pub pixel_size: PixelSize,
    /// In seconds.
    pub duration: Seconds,
    /// The file's type as its extension: "mp4", "png".
    pub file_extension: String,
}

impl OnDeviceOutput {
    /// The result's media.
    pub fn output_media(&self) -> OutputMedia {
        OutputMedia {
            duration: self.duration,
            pixel_size: self.pixel_size,
        }
    }
}
