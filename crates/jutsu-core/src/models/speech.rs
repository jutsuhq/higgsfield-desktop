//! Speech heard on-device, and the subtitles made from it.
//!
//! The macOS models (macOS 26's SpeechTranscriber and DictationTranscriber, and Parakeet Ultra through Core ML) run
//! only on macOS; what is ported of them is their names, the languages rules and the transcript values. Windows and
//! Linux hear speech with whisper.cpp (`SpeechModel::Whisper`, `WhisperModel`), which takes the built-in models' place.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::rules::{CueBuilder, SpeechSelection, SubtitleCue};
use crate::support::Seconds;
use crate::support::string_enum;
use crate::support::text::{is_punctuation, is_whitespace_or_newline, trim};

string_enum! {
    /// A speech model that runs on-device and turns the speech of a file's sound into timed words.
    pub enum SpeechModel {
        /// macOS 26's SpeechTranscriber, the built-in model for the locales it has.
        Transcriber = "speech_transcriber",
        /// macOS 26's DictationTranscriber, the built-in model for the locales SpeechTranscriber lacks.
        Dictation = "dictation",
        /// Parakeet Ultra (`ParakeetUltra`), a model the app downloads once, for 25 European languages.
        ParakeetUltra = "parakeet_ultra",
        /// whisper.cpp with `WhisperModel`, the built-in engine on Windows and Linux: the app downloads its model on
        /// the first run the person agreed to, as macOS downloads a built-in model.
        Whisper = "whisper",
    }
}

impl SpeechModel {
    /// One of macOS's own models, which take exactly the locales they list.
    pub fn is_built_in(self) -> bool {
        matches!(self, Self::Transcriber | Self::Dictation)
    }

    /// Whether a run downloads the model itself once the person agreed to it for the run's language (the built-in
    /// models and whisper), rather than Settings downloading it first (Parakeet Ultra).
    pub fn asks_consent(self) -> bool {
        self != Self::ParakeetUltra
    }
}

string_enum! {
    /// Which model makes a run's subtitles, as the person picks it.
    pub enum SubtitleEngine {
        /// The best model here for the language (`SpeechSelection::auto`).
        Auto = "auto",
        /// macOS's own models (`SpeechSelection::built_in`).
        BuiltIn = "built_in",
        /// Parakeet Ultra.
        Parakeet = "parakeet",
    }
}

/// A word as it was spoken, in seconds from the start of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TimedWord {
    /// The word.
    pub text: String,
    /// When it starts.
    pub start: Seconds,
    /// When it ends.
    pub end: Seconds,
}

impl TimedWord {
    /// `text` from `start` to `end`.
    pub fn new(text: impl Into<String>, start: Seconds, end: Seconds) -> Self {
        Self {
            text: text.into(),
            start,
            end,
        }
    }
}

/// What a speech model heard in a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Transcript {
    /// BCP 47, "en-US".
    pub language: String,
    /// The model that heard it.
    pub model: SpeechModel,
    /// The model's revision: another revision hears other words, so it is part of the cache key.
    pub revision: String,
    /// The words, in order.
    pub words: Vec<TimedWord>,
}

impl Transcript {
    /// The words as text, spaced as the language writes them.
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(WordSpacing::separator(&self.language))
    }
}

/// How a language writes its words apart: most with spaces; Chinese, Japanese, Cantonese, Thai, Lao, Khmer and
/// Burmese without.
pub struct WordSpacing;

impl WordSpacing {
    /// The languages written without spaces, by code.
    pub const UNSPACED: [&'static str; 7] = ["ja", "zh", "yue", "th", "lo", "km", "my"];

    /// What goes between two words of `language`: "" for a language written without spaces, else " ".
    pub fn separator(language: &str) -> &'static str {
        if Self::UNSPACED.contains(&SpeechSelection::language_code(language).as_str()) {
            ""
        } else {
            " "
        }
    }

    /// A model's timed runs of one result, as it writes them, as words. In a language written with spaces, a run
    /// that doesn't start with one, as a comma or the rest of a word, joins the word before it. In one written
    /// without, each run is a word of its own, and only punctuation joins the word before it.
    pub fn append(runs: &[TimedWord], words: &mut Vec<TimedWord>, language: &str) {
        let spaced = !Self::separator(language).is_empty();
        let mut starts_result = true;
        for run in runs {
            let trimmed = trim(&run.text);
            if trimmed.is_empty() {
                continue;
            }
            let continues = if spaced {
                run.text
                    .chars()
                    .next()
                    .is_some_and(|first| !is_whitespace_or_newline(first))
            } else {
                trimmed.chars().all(is_punctuation)
            };
            if !starts_result
                && continues
                && let Some(last) = words.last_mut()
            {
                last.text.push_str(trimmed);
                last.end = last.end.max(run.end);
            } else {
                words.push(TimedWord::new(trimmed, run.start, run.end.max(run.start)));
            }
            starts_result = false;
        }
    }
}

/// The subtitles of one version of a video, made from its own file. A later version starts without any.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Subtitles {
    /// What the model heard, as it heard it.
    pub transcript: Transcript,
    /// The cues as built from the transcript (`CueBuilder`) and then edited.
    pub cues: Vec<SubtitleCue>,
    /// When they were made.
    #[serde(with = "crate::support::date::rfc3339")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: OffsetDateTime,
}

impl Subtitles {
    /// The subtitles of `transcript`: `cues` when given, else the cues built from its words.
    pub fn new(transcript: Transcript, cues: Option<Vec<SubtitleCue>>, created_at: OffsetDateTime) -> Self {
        let cues = cues.unwrap_or_else(|| {
            CueBuilder::cues(&transcript.words, WordSpacing::separator(&transcript.language))
        });
        Self {
            transcript,
            cues,
            created_at,
        }
    }

    /// The transcript's language.
    pub fn language(&self) -> &str {
        &self.transcript.language
    }
}

/// The speech models this computer has, and the languages each takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechAvailability {
    /// BCP 47 identifiers by model; a model this computer lacks has none.
    pub languages: BTreeMap<SpeechModel, Vec<String>>,
    /// The languages whose model is here already, so a run needs no download.
    pub installed: BTreeMap<SpeechModel, BTreeSet<String>>,
}

impl SpeechAvailability {
    /// No model at all.
    pub fn none() -> Self {
        Self::default()
    }

    /// The built-in models take exactly the locales they list; Parakeet and whisper take a language by its code alone,
    /// so "de-AT" as well as "de".
    pub fn supports(&self, model: SpeechModel, language: &str) -> bool {
        let Some(listed) = self.languages.get(&model) else {
            return false;
        };
        let key = if model.is_built_in() {
            language.to_owned()
        } else {
            SpeechSelection::language_code(language)
        };
        listed.contains(&key)
    }

    /// Whether the model for `language` is here already.
    pub fn is_installed(&self, model: SpeechModel, language: &str) -> bool {
        let Some(installed) = self.installed.get(&model) else {
            return false;
        };
        let key = if model.is_built_in() {
            language.to_owned()
        } else {
            SpeechSelection::language_code(language)
        };
        installed.contains(&key)
    }
}

/// Why subtitles couldn't be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum TranscriptionError {
    /// The file is missing, or the app may no longer read it.
    #[error("cantOpenFile")]
    CantOpenFile,
    /// The file has no sound, only silence, or nothing the model took for speech.
    #[error("noSpeech")]
    NoSpeech,
    /// No model here takes the language.
    #[error("unsupportedLanguage")]
    UnsupportedLanguage,
    /// The language's model isn't here, and the person hasn't agreed to downloading it.
    #[error("modelNeeded")]
    ModelNeeded,
    /// Parakeet Ultra isn't downloaded; Settings › Subtitles downloads it.
    #[error("modelNotDownloaded")]
    ModelNotDownloaded,
    /// Too little free memory to load the model now, in megabytes.
    #[error("notEnoughMemory")]
    NotEnoughMemory {
        /// What it needs.
        needed_mb: i64,
        /// What is free.
        free_mb: i64,
    },
}
