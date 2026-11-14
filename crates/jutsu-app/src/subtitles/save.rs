//! Whether the editor's Done saves (`SubtitleSave.swift`): only over the subtitles it opened with.

use jutsu_core::Subtitles;

/// What Done does with the editor's cues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleSave {
    /// The version still has what the editor opened: save over it.
    Save,
    /// The subtitles were made again, or removed, while the editor was open: ask before replacing them.
    ChangedMeanwhile,
    /// The version is no longer in the library.
    VersionGone,
}

impl SubtitleSave {
    /// `stored` is the version's subtitles now, `None` when the version is gone.
    pub fn decide(opened: &Subtitles, stored: Option<Option<&Subtitles>>) -> Self {
        match stored {
            None => Self::VersionGone,
            Some(Some(stored)) if stored == opened => Self::Save,
            Some(_) => Self::ChangedMeanwhile,
        }
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::{SpeechModel, SubtitleCue, TimedWord, Transcript};
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn done_saves_only_over_what_the_editor_opened_with() {
        let opened = Subtitles::new(
            Transcript {
                language: "en-US".to_owned(),
                model: SpeechModel::Transcriber,
                revision: "1".to_owned(),
                words: vec![TimedWord::new("Hello", 0.0, 0.5)],
            },
            None,
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(
            SubtitleSave::decide(&opened, Some(Some(&opened))),
            SubtitleSave::Save
        );
        let mut remade = opened.clone();
        remade.cues = vec![SubtitleCue::new(0.0, 2.0, "Hello again")];
        assert_eq!(
            SubtitleSave::decide(&opened, Some(Some(&remade))),
            SubtitleSave::ChangedMeanwhile
        );
        assert_eq!(
            SubtitleSave::decide(&opened, Some(None)),
            SubtitleSave::ChangedMeanwhile
        );
        assert_eq!(SubtitleSave::decide(&opened, None), SubtitleSave::VersionGone);
    }
}
