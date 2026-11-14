//! The texts of subtitles made on this computer (`SubtitleContent.swift`, `OnDeviceContent.swift`).
//!
//! The macOS wording names "this Mac", macOS's own speech models and Parakeet Ultra; here the built-in engine is
//! whisper.cpp with `WhisperModel`, which Jutsu Studio downloads from Hugging Face, so those sentences say so.

use jutsu_core::{
    PortError, SpeechSelection, SubtitleCue, SubtitleEngine, Subtitles, TranscriptionError, WhisperModel,
};

use super::runs::SubtitleRunState;
use crate::settings::format_bytes;

pub const TITLE: &str = "Subtitles";
pub const VERSION_NOTE: &str =
    "The subtitles belong to this version of the item. A later version starts without them.";
pub const SOUND_NOTE: &str = "Only the sound is read; nothing leaves this computer.";
pub const MAKING: &str = "Making subtitles";
pub const FAILURE_TITLE: &str = "Can't make the subtitles";
pub const NO_SPEECH: &str = "No speech found";
pub const CANT_OPEN_FILE: &str =
    "Can't open the video's file. It was moved or deleted, or the app may no longer read it.";
pub const UNSUPPORTED_LANGUAGE: &str = "No speech model on this computer takes this language.";
pub const MODEL_NEEDED: &str =
    "The speech model for this language isn't on this computer yet. Agree to its download in Subtitles….";
pub const MODEL_CONSENT: &str = "Download the speech model";
pub const PARAKEET_NOT_DOWNLOADED: &str =
    "Parakeet Ultra isn't downloaded yet. Download it in Settings › Subtitles.";
pub const MODELS_FOLDER: &str = "Speech models";
pub const DOWNLOAD_FAILED_TITLE: &str = "Can't download Whisper Small";
pub const REMOVE_FAILED_TITLE: &str = "Can't remove Whisper Small";
pub const AUTO_NOTE: &str = "Auto hears a language with the best speech model on this computer for it; here that is \
     Whisper, which Jutsu Studio downloads once. The Subtitles sheet can pick another for a run.";
pub const WHISPER_SUMMARY: &str = "OpenAI's Whisper small, multilingual, in the ggml build of the whisper.cpp project, \
     quantized to 5 bits. Hears about 100 languages on this computer.";
pub const WHISPER_CREDIT: &str = "Whisper Small speech model: OpenAI Whisper, MIT License, ggml build by the whisper.cpp \
     project. Run with whisper.cpp, MIT License.";
pub const CHANGED_MEANWHILE_TITLE: &str = "These subtitles were made again while you edited them";
pub const CHANGED_MEANWHILE_MESSAGE: &str =
    "Replace the new subtitles with your edits, or keep the new ones and leave your edits.";
pub const VERSION_GONE_MESSAGE: &str = "The version these subtitles belong to is no longer in the library.";
pub const NOT_SAVED_TITLE: &str = "Can't save the subtitles";

/// `OnDeviceText.price`, `waiting` and `downloadingModel`.
pub const PRICE: &str = "Free · runs on this computer";
pub const WAITING: &str = "Waiting for the run before it";
pub const DOWNLOADING_MODEL: &str = "Downloading the model";

/// Before the first run in a language whose model isn't here: "… about 190 MB. It stays for later runs."
pub fn model_notice() -> String {
    format!(
        "Hearing this language needs a speech model Jutsu Studio downloads from Hugging Face once, about {}. It stays \
         for later runs.",
        format_bytes(WhisperModel::manifest().size())
    )
}

/// "190 MB download · 100 languages · MIT".
pub fn whisper_facts() -> String {
    [
        format!("{} download", format_bytes(WhisperModel::manifest().size())),
        format!("{} languages", WhisperModel::LANGUAGES.len()),
        WhisperModel::LICENSE.to_owned(),
    ]
    .join(" · ")
}

/// The languages whisper hears, by name, sorted.
pub fn whisper_languages() -> String {
    let mut names: Vec<&str> = WhisperModel::LANGUAGES
        .iter()
        .map(|code| WhisperModel::language_name(code))
        .collect();
    names.sort_unstable();
    and_list(&names)
}

/// "A, B, and C", as an English list.
pub fn and_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The picker's name of an engine. The built-in engine on this computer is whisper.
pub fn engine_title(engine: SubtitleEngine) -> &'static str {
    match engine {
        SubtitleEngine::Auto => "Auto",
        SubtitleEngine::BuiltIn => "Whisper",
        SubtitleEngine::Parakeet => "Parakeet Ultra",
    }
}

/// The engines the pickers offer: Parakeet Ultra runs only on macOS (Core ML).
pub const OFFERED_ENGINES: [SubtitleEngine; 2] = [SubtitleEngine::Auto, SubtitleEngine::BuiltIn];

/// "English (US)" for "en-US", "English" for "en", in English.
pub fn language_name(identifier: &str) -> String {
    let code = SpeechSelection::language_code(identifier);
    let name = WhisperModel::language_name(&code);
    if name == code {
        return identifier.to_owned();
    }
    let region = identifier
        .split(['-', '_'])
        .skip(1)
        .find(|part| part.len() == 2 || part.chars().all(|c| c.is_ascii_digit()));
    match region {
        Some(region) => format!("{name} ({})", region.to_uppercase()),
        None => name.to_owned(),
    }
}

/// "42%" for 0.42, rounded half up (`CostText.percent(fraction:)`).
pub fn percent(fraction: f64) -> String {
    #[allow(clippy::cast_possible_truncation, reason = "a percent")]
    let value = (fraction * 100.0).round() as i64;
    format!("{value}%")
}

/// "Making subtitles · 42%", or that it waits.
pub fn progress(state: SubtitleRunState) -> String {
    match state {
        SubtitleRunState::Waiting => format!("{MAKING} · {WAITING}"),
        SubtitleRunState::DownloadingModel { fraction } => {
            format!("{DOWNLOADING_MODEL} · {}", percent(fraction))
        }
        SubtitleRunState::Working { fraction } => format!("{MAKING} · {}", percent(fraction)),
    }
}

/// What a failed run tells the person.
pub fn message(error: &PortError) -> String {
    match error {
        PortError::Transcription(error) => transcription_message(*error),
        other => other.to_string(),
    }
}

fn transcription_message(error: TranscriptionError) -> String {
    match error {
        TranscriptionError::CantOpenFile => CANT_OPEN_FILE.to_owned(),
        TranscriptionError::NoSpeech => NO_SPEECH.to_owned(),
        TranscriptionError::UnsupportedLanguage => UNSUPPORTED_LANGUAGE.to_owned(),
        TranscriptionError::ModelNeeded => MODEL_NEEDED.to_owned(),
        TranscriptionError::ModelNotDownloaded => PARAKEET_NOT_DOWNLOADED.to_owned(),
        TranscriptionError::NotEnoughMemory { needed_mb, free_mb } => format!(
            "Parakeet Ultra needs about {needed_mb} MB of free memory, and {free_mb} MB are free now. Quit other apps \
             and try again."
        ),
    }
}

/// "0:01.2 – 0:03.4".
pub fn time_range(cue: &SubtitleCue) -> String {
    format!("{} – {}", time(cue.start), time(cue.end))
}

/// "1:02.3", or "1:01:02.3" past an hour.
pub fn time(seconds: f64) -> String {
    #[allow(clippy::cast_possible_truncation, reason = "tenths of a second")]
    let tenths = (seconds.max(0.0) * 10.0).round() as i64;
    let (minutes, secs, tenth) = (tenths / 600, tenths / 10 % 60, tenths % 10);
    let rest = format!("{secs:02}.{tenth}");
    if minutes >= 60 {
        format!("{}:{:02}:{rest}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}:{rest}")
    }
}

/// "English (US) · 24 cues".
pub fn summary(subtitles: &Subtitles) -> String {
    let count = subtitles.cues.len();
    let cues = if count == 1 {
        "1 cue".to_owned()
    } else {
        format!("{count} cues")
    };
    format!("{} · {cues}", language_name(subtitles.language()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_the_editor_shows_them() {
        assert_eq!(time(1.24), "0:01.2");
        assert_eq!(time(62.3), "1:02.3");
        assert_eq!(time(3662.3), "1:01:02.3");
        assert_eq!(time(-1.0), "0:00.0");
        assert_eq!(time_range(&SubtitleCue::new(1.2, 3.4, "Hi")), "0:01.2 – 0:03.4");
    }

    #[test]
    fn progress_names_the_step_and_its_share() {
        assert_eq!(
            progress(SubtitleRunState::Waiting),
            "Making subtitles · Waiting for the run before it"
        );
        assert_eq!(
            progress(SubtitleRunState::Working { fraction: 0.424 }),
            "Making subtitles · 42%"
        );
        assert_eq!(
            progress(SubtitleRunState::DownloadingModel { fraction: 0.5 }),
            "Downloading the model · 50%"
        );
    }

    #[test]
    fn languages_and_messages_read_in_english() {
        assert_eq!(language_name("en-US"), "English (US)");
        assert_eq!(language_name("de"), "German");
        assert_eq!(language_name("zh-Hans-CN"), "Chinese (CN)");
        assert_eq!(language_name("xx-YY"), "xx-YY");
        assert_eq!(
            message(&PortError::Transcription(TranscriptionError::NoSpeech)),
            NO_SPEECH
        );
        assert_eq!(message(&PortError::Other("disk full".to_owned())), "disk full");
        assert!(model_notice().contains("about 190 MB"));
        assert_eq!(whisper_facts(), "190 MB download · 100 languages · MIT");
        assert!(whisper_languages().starts_with("Afrikaans, Albanian, Amharic, "));
        assert_eq!(and_list(&["A", "B", "C"]), "A, B, and C");
    }
}
