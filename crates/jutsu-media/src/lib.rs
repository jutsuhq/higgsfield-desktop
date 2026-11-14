//! The ffmpeg media pipeline (Swift's `MediaKit`): probe, trim and transcode for upload, thumbnails and exact frames,
//! still images, content hashes, the source's sound put back, exports for editing and delivery, Join with Original,
//! the preview cut and model downloads. Every function runs the `ffmpeg` and `ffprobe` the caller supplies
//! (`FfmpegTools`) with arguments passed as a list, never through a shell.
//!
//! The on-device engines' cores (Smooth Motion, super-resolution upscale, speech transcription) come in a later phase.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "ported from Swift, which mixes Int and Double the same way; pixel sizes, seconds and byte counts stay far \
              inside both ranges"
)]
#![allow(
    clippy::doc_markdown,
    reason = "the docs keep the Swift original's wording and product names"
)]
#![allow(clippy::missing_panics_doc, reason = "nothing panics outside tests")]
#![allow(clippy::must_use_candidate, reason = "plain accessors and constructors")]

pub mod audio;
pub mod delivery;
pub mod download;
pub mod editing;
mod error;
pub mod frames;
pub mod hash;
pub mod join;
pub mod on_device;
mod prepare;
pub mod probe;
pub mod speech;
pub mod still;
mod tools;
pub mod video;

#[cfg(test)]
mod test_support;

pub use download::{ModelDownloadError, ModelDownloader};
pub use error::{MediaError, byte_count_string};
pub use frames::{FrameClip, FrameGrabError, FrameGrabReason, FrameRole, WrittenFrame, WrittenFrames};
pub use hash::{FileFingerprint, FileHashCache};
pub use on_device::{FrameUpscaling, OnDeviceEnhancer, RealEsrganEngine};
pub use prepare::MediaPreparer;
pub use probe::MediaInfo;
pub use speech::OnDeviceSpeech;
pub use tools::{FfmpegTools, H264Encoder};
