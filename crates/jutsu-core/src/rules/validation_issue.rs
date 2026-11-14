//! Why a job can't be submitted.

use crate::models::PixelSize;
use crate::support::Seconds;

/// Why a job can't be submitted. The app's wording for these lives in its localization, which isn't ported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValidationIssue {
    /// The selected range isn't finite or doesn't lie within the source file.
    SelectionOutsideSource,
    /// No source video, and the model can't make a clip without one.
    SourceRequired,
    /// A clip made without a source has neither a prompt nor an image to go on, or Edit or Extend has no prompt.
    PromptRequired,
    /// A frame side below one pixel.
    InvalidFrameSize(PixelSize),
    /// The length below the model's window.
    DurationTooShort(Seconds),
    /// The length above the model's window.
    DurationTooLong(Seconds),
    /// Fewer reference images than the mode takes.
    TooFewReferences {
        /// The fewest it takes.
        minimum: usize,
    },
    /// More reference images than the mode takes.
    TooManyReferences {
        /// The most it takes.
        maximum: usize,
    },
    /// Restyle without a preset.
    PresetRequired,
    /// Object swap frame below the API minimum and the user hasn't allowed an upscale.
    FrameTooSmall(PixelSize),
    /// Length counted in Unicode scalars, as the API's JSON schema does.
    PromptTooLong {
        /// The prompt's length.
        length: usize,
    },
    /// A model that voices a speech (`VideoControl::Speech`) has none.
    SpeechRequired,
    /// The speech below the model's window (`VideoModelDescriptor::speech_seconds`), in seconds.
    SpeechTooShort(Seconds),
    /// The speech above the model's window, in seconds.
    SpeechTooLong(Seconds),
    /// A model that voices a speech makes the clip from images alone and takes no source video.
    SourceNotTaken,
}
