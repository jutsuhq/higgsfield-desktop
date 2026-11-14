//! Which speech model hears a language.

use std::collections::BTreeSet;

use crate::models::{SpeechAvailability, SpeechModel, SubtitleEngine};

/// Which speech model hears a language, which languages the Subtitles sheet offers, and where a transcript is cached.
pub struct SpeechSelection;

impl SpeechSelection {
    /// Built-in: SpeechTranscriber for the locales it has, otherwise Dictation, otherwise whisper (the built-in engine
    /// on Windows and Linux); `None` when none takes it.
    pub fn built_in(language: &str, availability: &SpeechAvailability) -> Option<SpeechModel> {
        [
            SpeechModel::Transcriber,
            SpeechModel::Dictation,
            SpeechModel::Whisper,
        ]
        .into_iter()
        .find(|model| availability.supports(*model, language))
    }

    /// Auto: SpeechTranscriber for the locales it has, then Parakeet for its languages once it is downloaded, then
    /// Dictation, then whisper; Parakeet before it is downloaded last, so a language only it takes asks for the
    /// download.
    pub fn auto(language: &str, availability: &SpeechAvailability) -> Option<SpeechModel> {
        if availability.supports(SpeechModel::Transcriber, language) {
            return Some(SpeechModel::Transcriber);
        }
        let parakeet = availability.supports(SpeechModel::ParakeetUltra, language);
        if parakeet && availability.is_installed(SpeechModel::ParakeetUltra, language) {
            return Some(SpeechModel::ParakeetUltra);
        }
        if availability.supports(SpeechModel::Dictation, language) {
            return Some(SpeechModel::Dictation);
        }
        if availability.supports(SpeechModel::Whisper, language) {
            return Some(SpeechModel::Whisper);
        }
        parakeet.then_some(SpeechModel::ParakeetUltra)
    }

    /// The model `engine` hears `language` with; `None` when it doesn't take the language. Parakeet is named whether
    /// or not it is downloaded yet.
    pub fn model(
        engine: SubtitleEngine,
        language: &str,
        availability: &SpeechAvailability,
    ) -> Option<SpeechModel> {
        match engine {
            SubtitleEngine::Auto => Self::auto(language, availability),
            SubtitleEngine::BuiltIn => Self::built_in(language, availability),
            SubtitleEngine::Parakeet => availability
                .supports(SpeechModel::ParakeetUltra, language)
                .then_some(SpeechModel::ParakeetUltra),
        }
    }

    /// Every model's languages, once each, in identifier order: the built-in models' locales (whisper's codes among
    /// them), and Parakeet's languages that no built-in locale has, by their code.
    pub fn languages(availability: &SpeechAvailability) -> Vec<String> {
        let locales: BTreeSet<String> = SpeechModel::ALL
            .iter()
            .filter(|model| **model != SpeechModel::ParakeetUltra)
            .flat_map(|model| availability.languages.get(model).cloned().unwrap_or_default())
            .collect();
        let covered: BTreeSet<String> = locales.iter().map(|locale| Self::language_code(locale)).collect();
        let others = availability
            .languages
            .get(&SpeechModel::ParakeetUltra)
            .into_iter()
            .flatten()
            .filter(|language| !covered.contains(*language))
            .cloned();
        locales
            .into_iter()
            .chain(others)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The language the sheet starts on: the last one used when it is offered, else the system's first preferred
    /// language that is, matched exactly and then by its language alone ("en-GB" takes "en-US"), else the first.
    /// `system` is the system's preferred languages, as `Locale.preferredLanguages` lists them.
    pub fn preferred(languages: &[String], last: Option<&str>, system: &[String]) -> Option<String> {
        if let Some(last) = last
            && languages.iter().any(|language| language == last)
        {
            return Some(last.to_owned());
        }
        for preferred in system {
            if let Some(exact) = languages
                .iter()
                .find(|language| language.to_lowercase() == preferred.to_lowercase())
            {
                return Some(exact.clone());
            }
            let code = Self::language_code(preferred);
            if let Some(same) = languages
                .iter()
                .find(|language| Self::language_code(language) == code)
            {
                return Some(same.clone());
            }
        }
        languages.first().cloned()
    }

    /// "en" of "en-US", "zh" of "zh-Hans-CN".
    pub fn language_code(identifier: &str) -> String {
        identifier
            .split(['-', '_'])
            .find(|part| !part.is_empty())
            .unwrap_or(identifier)
            .to_lowercase()
    }

    /// The name a transcript is cached under: the same file content heard in the same language by the same model
    /// revision gives the same words. Only file-name-safe characters.
    pub fn cache_key(content_hash: &str, language: &str, model: SpeechModel, revision: &str) -> String {
        [content_hash, language, model.raw_value(), revision]
            .map(safe)
            .join("_")
    }
}

fn safe(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_alphabetic() || c.is_numeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
