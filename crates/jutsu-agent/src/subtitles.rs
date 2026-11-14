//! `subtitles`: the Subtitles sheet and editor of a video's current version, free on this computer.

use crate::{Uuid, names, values::wire_enum};
use serde::{Deserialize, Serialize};

wire_enum! {
    pub enum AgentSubtitlesAction {
        /// Hears the item's speech and saves the cues on its current version: a run, followed with `jobs`.
        Make = "make",
        /// The current version's cues, and with `format` their file's text.
        Get = "get",
        /// Writes the cues as a file into the output folder's Exports folder.
        Export = "export",
    }
}

wire_enum! {
    /// The speech models `make` takes, by the names agents use.
    pub enum AgentSubtitleEngine { Auto = "auto", BuiltIn = "built_in", Parakeet = "parakeet" }
}

wire_enum! {
    pub enum AgentSubtitleFormat { Srt = "srt", Vtt = "vtt" }
}

/// `subtitles` on one video of the Library. `make` is idempotent by its request ID and free; `get` and `export` read
/// what the current version has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitlesCommand {
    pub action: AgentSubtitlesAction,
    /// A video's Library ID.
    pub item: Uuid,
    pub request_id: Option<String>,
    pub fingerprint: Option<String>,
    /// make: BCP 47, such as en-US, or a language code, such as de.
    pub language: Option<String>,
    pub engine: Option<AgentSubtitleEngine>,
    /// get: adds the cues' file text; export: the file's format, needed.
    pub format: Option<AgentSubtitleFormat>,
}

impl SubtitlesCommand {
    /// The longest language identifier taken.
    pub const MAX_LANGUAGE_LENGTH: usize = 35;

    pub fn problem(&self) -> Option<String> {
        match self.action {
            AgentSubtitlesAction::Make => {
                if !self.request_id.as_deref().is_some_and(names::is_valid_request_id) {
                    return Some(names::REQUEST_ID_RULE.to_owned());
                }
                if !self
                    .fingerprint
                    .as_deref()
                    .is_some_and(names::is_valid_fingerprint)
                {
                    return Some("fingerprint is 64 lowercase hex digits".to_owned());
                }
                if self.format.is_some() {
                    return Some("make takes no format; get and export do".to_owned());
                }
                if self
                    .language
                    .as_deref()
                    .is_some_and(|language| !Self::is_language(language))
                {
                    return Some(
                        "language is a BCP 47 identifier such as en-US, or a language code such as de"
                            .to_owned(),
                    );
                }
            }
            AgentSubtitlesAction::Get | AgentSubtitlesAction::Export => {
                let action = self.action.as_str();
                let extra = [
                    ("request_id", self.request_id.is_some()),
                    ("language", self.language.is_some()),
                    ("engine", self.engine.is_some()),
                ];
                if let Some((name, _)) = extra.iter().find(|(_, sent)| *sent) {
                    return Some(format!("{action} takes no {name}; make does"));
                }
                if self.fingerprint.is_some() {
                    return Some(format!("{action} takes no fingerprint"));
                }
                if self.action == AgentSubtitlesAction::Export && self.format.is_none() {
                    return Some("export needs format: srt or vtt".to_owned());
                }
            }
        }
        None
    }

    /// Letters, digits, `-` and `_`, starting with a letter.
    pub fn is_language(text: &str) -> bool {
        (2..=Self::MAX_LANGUAGE_LENGTH).contains(&text.len())
            && text.chars().next().is_some_and(char::is_alphabetic)
            && text
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    }
}

/// One cue, in seconds from the start of the video.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentCue {
    pub start: f64,
    pub end: f64,
    /// Its lines, joined by a newline.
    pub text: String,
}

/// `get` and `export`: the subtitles of the item's current version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSubtitles {
    pub item: Uuid,
    pub version_id: Uuid,
    /// BCP 47.
    pub language: String,
    /// The model that heard them: `speech_transcriber`, dictation or `parakeet_ultra`.
    pub model: String,
    pub cues: Vec<AgentCue>,
    pub text: Option<String>,
    pub file: Option<String>,
}

/// One speech model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSubtitleEngineState {
    /// `built_in` or parakeet.
    pub engine: String,
    pub available: bool,
    pub languages: Vec<String>,
    pub model: String,
    pub needs_consent: Option<Vec<String>>,
}

/// What `capabilities` says of subtitles on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSubtitleCapabilities {
    /// The engine `make` uses when it names none: auto, `built_in` or parakeet.
    pub default_engine: String,
    pub engines: Vec<AgentSubtitleEngineState>,
}
