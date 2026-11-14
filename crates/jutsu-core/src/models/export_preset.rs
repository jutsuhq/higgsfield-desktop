//! How results are written for editing and delivery.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::{CropAspect, MaterialKind, PixelSize};
use crate::rules::SubtitleFormat;
use crate::support::codable::Unlabeled;
use crate::support::string_enum;

string_enum! {
    /// How a result is re-encoded for an editing timeline. Every preset writes a constant frame rate.
    pub enum ExportPreset {
        /// Adobe Premiere Pro.
        Premiere = "premiere",
        /// DaVinci Resolve.
        DavinciResolve = "davinciResolve",
        /// Final Cut Pro.
        FinalCutPro = "finalCutPro",
        /// High-bitrate H.264 for any editor.
        EditingH264 = "editingH264",
    }
}

string_enum! {
    /// The video codec an export preset writes.
    pub enum VideoCodec {
        /// Apple ProRes 422.
        ProRes422 = "proRes422",
        /// High-bitrate H.264, for editors and machines without ProRes decoding.
        H264 = "h264",
    }
}

string_enum! {
    /// The audio codec an export preset writes.
    pub enum AudioCodec {
        /// Uncompressed PCM.
        LinearPcm = "linearPCM",
        /// AAC.
        Aac = "aac",
    }
}

string_enum! {
    /// The file container an export preset writes.
    pub enum Container {
        /// QuickTime, `.mov`.
        QuickTime = "quickTime",
        /// MPEG-4, `.mp4`.
        Mpeg4 = "mpeg4",
    }
}

impl Container {
    /// "mov" or "mp4".
    pub fn file_extension(self) -> &'static str {
        match self {
            Self::QuickTime => "mov",
            Self::Mpeg4 => "mp4",
        }
    }
}

impl ExportPreset {
    /// The three NLEs all import ProRes 422 with PCM audio natively on macOS.
    pub fn video_codec(self) -> VideoCodec {
        match self {
            Self::Premiere | Self::DavinciResolve | Self::FinalCutPro => VideoCodec::ProRes422,
            Self::EditingH264 => VideoCodec::H264,
        }
    }

    /// PCM with ProRes, AAC with H.264.
    pub fn audio_codec(self) -> AudioCodec {
        match self.video_codec() {
            VideoCodec::ProRes422 => AudioCodec::LinearPcm,
            VideoCodec::H264 => AudioCodec::Aac,
        }
    }

    /// QuickTime with ProRes, MPEG-4 with H.264.
    pub fn container(self) -> Container {
        match self.video_codec() {
            VideoCodec::ProRes422 => Container::QuickTime,
            VideoCodec::H264 => Container::Mpeg4,
        }
    }

    /// The container's extension.
    pub fn file_extension(self) -> &'static str {
        self.container().file_extension()
    }
}

/// One frame's duration as a rational, `value / timescale` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameDuration {
    /// The numerator.
    pub value: i64,
    /// The denominator.
    pub timescale: i32,
}

impl FrameDuration {
    /// 30 fps, for a rate that can't be read.
    pub const FALLBACK: Self = Self {
        value: 1,
        timescale: 30,
    };
    /// Bases of the NTSC rates, which run at `base * 1000 / 1001` frames per second.
    pub const NTSC_BASES: [i32; 5] = [24, 30, 48, 60, 120];

    /// The constant frame duration for a nominal rate as AVFoundation reports it. NTSC rates (23.976, 29.97,
    /// 59.94…) stay exact as 1001/24000 etc.; rounding them would drift a frame every ~33 s against the source in a
    /// timeline.
    pub fn constant(nominal_rate: f64) -> Self {
        if !nominal_rate.is_finite() || !(1.0..=240.0).contains(&nominal_rate) {
            return Self::FALLBACK;
        }
        for base in Self::NTSC_BASES {
            if (nominal_rate - f64::from(base) * 1000.0 / 1001.0).abs() < 0.01 {
                return Self {
                    value: 1001,
                    timescale: base * 1000,
                };
            }
        }
        Self {
            value: 1,
            timescale: nominal_rate.round() as i32,
        }
    }
}

string_enum! {
    /// The file format of a still image output.
    pub enum ImageFormat {
        /// JPEG.
        Jpeg = "jpeg",
        /// PNG.
        Png = "png",
        /// HEIC.
        Heic = "heic",
    }
}

impl ImageFormat {
    /// "jpg", "png" or "heic".
    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Heic => "heic",
        }
    }

    /// The uniform type ImageIO writes.
    pub fn type_identifier(self) -> &'static str {
        match self {
            Self::Jpeg => "public.jpeg",
            Self::Png => "public.png",
            Self::Heic => "public.heic",
        }
    }

    /// Only PNG keeps transparency; JPEG and HEIC are flattened onto the preset's background.
    pub fn keeps_alpha(self) -> bool {
        self == Self::Png
    }

    /// Whether `quality` applies.
    pub fn is_lossy(self) -> bool {
        self != Self::Png
    }
}

string_enum! {
    /// The color space a still image is written in.
    #[derive(Default)]
    pub enum ImageColorSpace {
        /// sRGB.
        #[default]
        Srgb = "sRGB",
        /// Display P3.
        DisplayP3 = "displayP3",
    }
}

/// How a still image is written for a delivery set: format, size, quality and color space. Video outputs use
/// `ExportPreset`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ImageExportPreset {
    /// The format.
    pub format: ImageFormat,
    /// The long side in pixels after the crop; `None` keeps the cropped size. Never upscales.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_edge: Option<i64>,
    /// 0…1, for JPEG and HEIC.
    pub quality: f64,
    /// The color space.
    pub color_space: ImageColorSpace,
    /// What transparent pixels become in a format without alpha.
    pub background: RgbColor,
}

impl ImageExportPreset {
    /// `format` at full size, quality 0.9, sRGB, on white.
    pub fn new(format: ImageFormat) -> Self {
        Self {
            format,
            long_edge: None,
            quality: 0.9,
            color_space: ImageColorSpace::Srgb,
            background: RgbColor::WHITE,
        }
    }

    /// The pixel size written for a cropped picture of `size`: scaled down to `long_edge`, never up.
    pub fn output_size(&self, size: PixelSize) -> PixelSize {
        let Some(long_edge) = self.long_edge else {
            return size;
        };
        let longest = size.width.max(size.height);
        if long_edge <= 0 || !size.is_valid() || longest <= long_edge {
            return size;
        }
        let scale = long_edge as f64 / longest as f64;
        PixelSize::new(
            ((size.width as f64 * scale).round() as i64).max(1),
            ((size.height as f64 * scale).round() as i64).max(1),
        )
    }
}

/// An opaque color, each component 0…1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RgbColor {
    /// Red.
    pub red: f64,
    /// Green.
    pub green: f64,
    /// Blue.
    pub blue: f64,
}

impl RgbColor {
    /// White.
    pub const WHITE: Self = Self {
        red: 1.0,
        green: 1.0,
        blue: 1.0,
    };
    /// Black.
    pub const BLACK: Self = Self {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
    };
}

string_enum! {
    /// What a video output does with the subtitles of each video's version.
    pub enum DeliverySubtitles {
        /// An `.srt` and a `.vtt` next to the video, under its name, timed as the video is.
        Files = "files",
        /// Drawn into the video's frames.
        BurnIn = "burn_in",
    }
}

/// An output's preset: an image one or a video one. Coded as Swift codes the enum, `{"image":{"_0":{…}}}`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = DeliveryPresetRepr))]
#[serde(into = "DeliveryPresetRepr", from = "DeliveryPresetRepr")]
pub enum DeliveryPreset {
    /// A still image preset.
    Image(ImageExportPreset),
    /// A video preset.
    Video(ExportPreset),
}

/// The stored shape of a `DeliveryPreset`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "DeliveryPreset")]
pub enum DeliveryPresetRepr {
    /// A still image preset.
    Image(Unlabeled<ImageExportPreset>),
    /// A video preset.
    Video(Unlabeled<ExportPreset>),
}

impl From<DeliveryPreset> for DeliveryPresetRepr {
    fn from(preset: DeliveryPreset) -> Self {
        match preset {
            DeliveryPreset::Image(value) => Self::Image(Unlabeled { value }),
            DeliveryPreset::Video(value) => Self::Video(Unlabeled { value }),
        }
    }
}

impl From<DeliveryPresetRepr> for DeliveryPreset {
    fn from(repr: DeliveryPresetRepr) -> Self {
        match repr {
            DeliveryPresetRepr::Image(Unlabeled { value }) => Self::Image(value),
            DeliveryPresetRepr::Video(Unlabeled { value }) => Self::Video(value),
        }
    }
}

/// One output of a delivery set: an image or video preset, and the frame every item is cropped to. Image outputs
/// apply to the set's images, video outputs to its videos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DeliveryOutput {
    /// Its identity.
    pub id: Uuid,
    /// The output's part of the file names, e.g. "web"; empty uses `summary`.
    pub name: String,
    /// The preset.
    pub preset: DeliveryPreset,
    /// `None` keeps each item's own frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<CropAspect>,
    /// A video output's subtitles; `None` for none, and for an image output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<DeliverySubtitles>,
}

impl DeliveryOutput {
    /// A new, unnamed output of `preset` that keeps each item's frame.
    pub fn new(preset: DeliveryPreset) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            preset,
            aspect: None,
            subtitles: None,
        }
    }

    /// The subtitle files written next to a video of this output: none unless its subtitles are `Files`.
    pub fn subtitle_formats(&self) -> Vec<SubtitleFormat> {
        if self.kind() == MaterialKind::Video && self.subtitles == Some(DeliverySubtitles::Files) {
            SubtitleFormat::ALL.to_vec()
        } else {
            Vec::new()
        }
    }

    /// Whether it writes images or videos.
    pub fn kind(&self) -> MaterialKind {
        match self.preset {
            DeliveryPreset::Image(_) => MaterialKind::Image,
            DeliveryPreset::Video(_) => MaterialKind::Video,
        }
    }

    /// The extension of the files it writes.
    pub fn file_extension(&self) -> &'static str {
        match self.preset {
            DeliveryPreset::Image(image) => image.format.file_extension(),
            DeliveryPreset::Video(video) => video.file_extension(),
        }
    }

    /// "jpg-2048-4x5": what a file name says when the output has no name of its own.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = match self.preset {
            DeliveryPreset::Image(image) => {
                let mut format = vec![image.format.file_extension().to_owned()];
                format.extend(image.long_edge.map(|edge| edge.to_string()));
                format
            }
            DeliveryPreset::Video(video) => vec![video.raw_value().to_owned()],
        };
        parts.extend(self.aspect.map(|aspect| aspect.key()));
        if self.kind() == MaterialKind::Video && self.subtitles == Some(DeliverySubtitles::BurnIn) {
            parts.push("subtitled".to_owned());
        }
        parts.join("-")
    }
}
