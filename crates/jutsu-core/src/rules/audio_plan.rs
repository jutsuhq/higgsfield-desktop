//! What to do with the sound of a downloaded result.

use crate::models::AudioOption;
use crate::support::Seconds;
use crate::support::range::ClosedRange;

/// What to do with the sound of a downloaded result. Times are in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioPlan {
    /// Leave the file as it is.
    Keep,
    /// Write the video again without its audio track.
    RemoveAudio,
    /// Put `range` of the source's audio at the start of the result. `fade_out` is in result time.
    AddSource {
        /// The part of the source's audio.
        range: ClosedRange<Seconds>,
        /// Where it fades out, in result time.
        fade_out: Option<ClosedRange<Seconds>>,
    },
}

impl AudioPlan {
    /// How long a fade-out lasts.
    pub const FADE_OUT_DURATION: Seconds = 1.0;

    /// A result that already has sound keeps it, unless muted, and one whose sound the model was asked to make
    /// (`Generated`) is left as it came. The source audio is cut to the result's length and never stretched, so a
    /// result longer than the range goes silent where the audio ends. `source_audio_end` is where the source's
    /// audio track ends; `None` when it has none.
    pub fn make(
        option: AudioOption,
        source_range: ClosedRange<Seconds>,
        source_audio_end: Option<Seconds>,
        result_duration: Seconds,
        result_has_audio: bool,
    ) -> Self {
        if option == AudioOption::Mute {
            return if result_has_audio {
                Self::RemoveAudio
            } else {
                Self::Keep
            };
        }
        if option == AudioOption::Generated {
            return Self::Keep;
        }
        let Some(source_audio_end) = source_audio_end else {
            return Self::Keep;
        };
        if result_has_audio || result_duration <= 0.0 {
            return Self::Keep;
        }
        let start = source_range.lower().max(0.0);
        let end = source_range
            .upper()
            .min(source_audio_end)
            .min(start + result_duration);
        if end <= start {
            return Self::Keep;
        }
        let length = end - start;
        let fade_out = (option == AudioOption::FadeOut)
            .then(|| ClosedRange::new((length - Self::FADE_OUT_DURATION).max(0.0), length));
        Self::AddSource {
            range: ClosedRange::new(start, end),
            fade_out,
        }
    }
}
