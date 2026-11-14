//! Why a media step failed (Swift's `MediaError`), with the macOS app's wording.

use std::path::{Path, PathBuf};

use jutsu_core::{DeliverySetError, ImageFormat, PortError};

use crate::prepare::MediaPreparer;

/// Why a media step failed. The messages are the macOS app's; the cases after `EngineUnavailable` have no Swift
/// counterpart because AVFoundation never fails that way.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// No video stream ffmpeg can decode. The user has to convert the file first.
    #[error("This video's format can't be read. Convert it to MP4 or MOV first.")]
    UnsupportedFormat,
    /// The file can't be read as an image.
    #[error("This image can't be read. Save it as JPEG or PNG and add it again.")]
    UnreadableImage,
    /// The selected range is empty or lies outside the file.
    #[error("The selected range lies outside the video. Select it again.")]
    InvalidRange,
    /// Export or encoding failed; carries the underlying error's description.
    #[error("The file couldn't be converted: {0}")]
    ExportFailed(String),
    /// Even the re-encode at the computed bitrate came out over the upload limit.
    #[error(
        "Even re-encoded, this video is {}, over the {} upload limit. Select a shorter range.",
        byte_count_string(*byte_count),
        byte_count_string(MediaPreparer::FILE_LENGTH_LIMIT)
    )]
    TooLarge {
        /// The re-encoded file's size.
        byte_count: i64,
    },
    /// This computer has no engine for an on-device enhancement, or it won't take this file's frames.
    #[error("This Mac can't do this to this file.")]
    EngineUnavailable,
    /// The on-device engine can't run on this computer at all, for this reason: no build for its platform, or no
    /// Vulkan driver for Real-ESRGAN.
    #[error("Upscale on this computer can't run here: {0}")]
    EngineUnavailableBecause(String),
    /// The ffmpeg in use has no H.264 encoder that works on this computer.
    #[error("This computer's ffmpeg has no working H.264 encoder.")]
    NoH264Encoder,
    /// The ffmpeg in use has neither the `subtitles` (libass) nor the `drawtext` (FreeType) filter, so cues can't be
    /// drawn into a video's frames.
    #[error(
        "Subtitles can't be burned in on this computer: its ffmpeg can't draw text. Write them as SRT and VTT files instead."
    )]
    BurnInUnavailable,
    /// No encoder for this image format on this platform (HEIC).
    #[error("{} images can't be written on this computer. Choose JPEG or PNG.", .0.file_extension().to_uppercase())]
    FormatUnavailable(ImageFormat),
    /// A file is already at the path, and nothing is written over it.
    #[error("The file “{}” couldn't be saved because a file with the same name already exists.", file_name(.0))]
    FileExists(PathBuf),
    /// Reading or writing a file failed: the system's description.
    #[error("{0}")]
    Io(String),
    /// A delivery set's rule refused the export.
    #[error(transparent)]
    DeliverySet(#[from] DeliverySetError),
    /// The work was cancelled.
    #[error("cancelled")]
    Cancelled,
}

impl From<std::io::Error> for MediaError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<MediaError> for PortError {
    fn from(error: MediaError) -> Self {
        match error {
            MediaError::Cancelled => Self::Cancelled,
            other => Self::Other(other.to_string()),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A byte count as Foundation's `ByteCountFormatter` writes it in its `.file` style: decimal units, a whole number of
/// KB, one fraction digit of MB and two of GB and above, trailing zeros left out.
pub fn byte_count_string(byte_count: i64) -> String {
    let bytes = byte_count.unsigned_abs() as f64;
    let sign = if byte_count < 0 { "-" } else { "" };
    if byte_count == 0 {
        return "Zero KB".to_owned();
    }
    if bytes < 1000.0 {
        let unit = if byte_count.unsigned_abs() == 1 {
            "byte"
        } else {
            "bytes"
        };
        return format!("{sign}{bytes} {unit}");
    }
    let units = [("KB", 0), ("MB", 1), ("GB", 2), ("TB", 2), ("PB", 2)];
    let mut value = bytes / 1000.0;
    let mut index = 0;
    while value >= 999.5 && index + 1 < units.len() {
        value /= 1000.0;
        index += 1;
    }
    let (unit, digits) = units[index];
    let mut text = format!("{value:.digits$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    format!("{sign}{text} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed job shows the description, so it names sizes the way Finder does, never the case.
    #[test]
    fn a_too_large_video_names_its_size_and_the_limit() {
        let text = MediaError::TooLarge {
            byte_count: 220_000_000,
        }
        .to_string();
        assert!(text.contains("220 MB"), "{text}");
        assert!(text.contains("209.7 MB"), "{text}");
        assert!(!text.contains("TooLarge"));
    }

    #[test]
    fn byte_counts_read_as_finder_writes_them() {
        assert_eq!(byte_count_string(0), "Zero KB");
        assert_eq!(byte_count_string(512), "512 bytes");
        assert_eq!(byte_count_string(2_048), "2 KB");
        assert_eq!(byte_count_string(1_500_000), "1.5 MB");
        assert_eq!(byte_count_string(2_340_000_000), "2.34 GB");
    }
}
