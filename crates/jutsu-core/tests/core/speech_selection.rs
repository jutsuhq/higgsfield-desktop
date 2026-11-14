//! `SpeechSelectionTests.swift`.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use jutsu_core::*;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::common::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn availability() -> SpeechAvailability {
    SpeechAvailability {
        languages: BTreeMap::from([
            (SpeechModel::Transcriber, strings(&["en-US", "de-DE"])),
            (SpeechModel::Dictation, strings(&["en-US", "ru-RU"])),
        ]),
        installed: BTreeMap::from([(SpeechModel::Transcriber, set(&["en-US"]))]),
    }
}

#[test]
fn built_in_prefers_the_transcriber_and_falls_back_to_dictation() {
    let availability = availability();
    assert_eq!(
        SpeechSelection::built_in("en-US", &availability),
        Some(SpeechModel::Transcriber)
    );
    assert_eq!(
        SpeechSelection::built_in("ru-RU", &availability),
        Some(SpeechModel::Dictation)
    );
    assert_eq!(SpeechSelection::built_in("lv-LV", &availability), None);
    assert_eq!(
        SpeechSelection::built_in("en-US", &SpeechAvailability::none()),
        None
    );
}

#[test]
fn the_sheet_offers_every_language_once() {
    assert_eq!(
        SpeechSelection::languages(&availability()),
        ["de-DE", "en-US", "ru-RU"]
    );
}

#[test]
fn parakeet_takes_a_language_by_its_code_and_adds_only_the_ones_the_built_in_models_lack() {
    let mut both = availability();
    both.languages
        .insert(SpeechModel::ParakeetUltra, strings(&["de", "lv"]));
    both.installed.insert(SpeechModel::ParakeetUltra, set(&["lv"]));

    assert_eq!(
        SpeechSelection::model(SubtitleEngine::Parakeet, "de-AT", &both),
        Some(SpeechModel::ParakeetUltra)
    );
    assert_eq!(
        SpeechSelection::model(SubtitleEngine::Parakeet, "ru-RU", &both),
        None
    );
    assert_eq!(
        SpeechSelection::model(SubtitleEngine::Parakeet, "de", &availability()),
        None
    );
    assert_eq!(SpeechSelection::model(SubtitleEngine::BuiltIn, "lv", &both), None);
    assert!(both.is_installed(SpeechModel::ParakeetUltra, "lv-LV"));
    assert!(!both.is_installed(SpeechModel::ParakeetUltra, "de-DE"));
    assert_eq!(
        SpeechSelection::languages(&both),
        ["de-DE", "en-US", "lv", "ru-RU"]
    );
}

#[test]
fn auto_prefers_the_transcriber_then_a_downloaded_parakeet_then_dictation() {
    let mut all = availability();
    all.languages
        .insert(SpeechModel::ParakeetUltra, strings(&["en", "ru", "lv"]));
    let auto =
        |language, all: &SpeechAvailability| SpeechSelection::model(SubtitleEngine::Auto, language, all);
    assert_eq!(auto("en-US", &all), Some(SpeechModel::Transcriber));
    assert_eq!(auto("ru-RU", &all), Some(SpeechModel::Dictation));
    // Before the download Parakeet comes last, so a language only it takes still names it.
    assert_eq!(auto("lv", &all), Some(SpeechModel::ParakeetUltra));
    assert_eq!(auto("ja-JP", &all), None);
    all.installed
        .insert(SpeechModel::ParakeetUltra, set(&["en", "ru", "lv"]));
    assert_eq!(auto("ru-RU", &all), Some(SpeechModel::ParakeetUltra));
    assert_eq!(auto("en-US", &all), Some(SpeechModel::Transcriber));
}

#[test]
fn the_last_language_comes_first_then_the_systems_then_the_first() {
    let languages = strings(&["de-DE", "en-US", "ru-RU"]);
    let preferred = |languages: &[String], last, system: &[&str]| {
        SpeechSelection::preferred(languages, last, &strings(system))
    };
    assert_eq!(
        preferred(&languages, Some("ru-RU"), &["en-US"]).as_deref(),
        Some("ru-RU")
    );
    assert_eq!(
        preferred(&languages, Some("fr-FR"), &["en-US"]).as_deref(),
        Some("en-US")
    );
    assert_eq!(
        preferred(&languages, None, &["en-GB", "de"]).as_deref(),
        Some("en-US")
    );
    assert_eq!(
        preferred(&languages, None, &["fr-FR", "de"]).as_deref(),
        Some("de-DE")
    );
    assert_eq!(preferred(&languages, None, &["fr-FR"]).as_deref(), Some("de-DE"));
    assert_eq!(preferred(&[], Some("en-US"), &["en-US"]), None);
}

#[test]
fn the_cache_key_changes_with_each_part_and_is_a_file_name() {
    let name = SpeechSelection::cache_key("hash", "en-US", SpeechModel::Transcriber, "r1");
    assert_eq!(name, "hash_en-US_speech-transcriber_r1");
    let mut all: HashSet<String> = [
        SpeechSelection::cache_key("other", "en-US", SpeechModel::Transcriber, "r1"),
        SpeechSelection::cache_key("hash", "en-GB", SpeechModel::Transcriber, "r1"),
        SpeechSelection::cache_key("hash", "en-US", SpeechModel::Dictation, "r1"),
        SpeechSelection::cache_key("hash", "en-US", SpeechModel::Transcriber, "r2"),
    ]
    .into();
    all.insert(name);
    assert_eq!(all.len(), 5);
    assert!(
        !SpeechSelection::cache_key("a/b", "x:y", SpeechModel::Dictation, "1 2").contains(['/', ':', ' '])
    );
}

#[test]
fn a_version_without_subtitles_still_decodes() {
    let mut version = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::UNIX_EPOCH);
    version.file_url = Some(file_url("/tmp/clip.mp4"));
    let mut json = serde_json::to_value(&version).unwrap();
    json.as_object_mut().unwrap().remove("subtitles");
    let decoded: MaterialVersion = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.subtitles, None);
}

// Windows and Linux: whisper is the built-in engine, and takes a language by its code.

fn whisper() -> SpeechAvailability {
    SpeechAvailability {
        languages: BTreeMap::from([(SpeechModel::Whisper, strings(&WhisperModel::LANGUAGES))]),
        installed: BTreeMap::new(),
    }
}

#[test]
fn whisper_is_the_built_in_model_where_macos_has_none() {
    let availability = whisper();
    for engine in [SubtitleEngine::Auto, SubtitleEngine::BuiltIn] {
        assert_eq!(
            SpeechSelection::model(engine, "de-AT", &availability),
            Some(SpeechModel::Whisper)
        );
        assert_eq!(
            SpeechSelection::model(engine, "yue", &availability),
            Some(SpeechModel::Whisper)
        );
        assert_eq!(SpeechSelection::model(engine, "xx-YY", &availability), None);
    }
    assert_eq!(
        SpeechSelection::model(SubtitleEngine::Parakeet, "de", &availability),
        None
    );
    assert!(SpeechModel::Whisper.asks_consent() && !SpeechModel::Whisper.is_built_in());
    assert!(!SpeechModel::ParakeetUltra.asks_consent());
    let languages = SpeechSelection::languages(&availability);
    assert_eq!(languages.len(), 100);
    assert_eq!(languages.first().map(String::as_str), Some("af"));
}

#[test]
fn whisper_follows_the_macos_models_in_auto() {
    let mut both = availability();
    both.languages
        .insert(SpeechModel::Whisper, strings(&["en", "lv", "ru"]));
    assert_eq!(
        SpeechSelection::auto("en-US", &both),
        Some(SpeechModel::Transcriber)
    );
    assert_eq!(
        SpeechSelection::auto("ru-RU", &both),
        Some(SpeechModel::Dictation)
    );
    assert_eq!(SpeechSelection::auto("lv", &both), Some(SpeechModel::Whisper));
}

#[test]
fn the_whisper_manifest_is_pinned() {
    let manifest = WhisperModel::manifest();
    assert_eq!(manifest.size(), 190_085_487);
    assert_eq!(
        manifest.url(&manifest.files[0]).map(|url| url.to_string()),
        Some(format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/{}/ggml-small-q5_1.bin",
            WhisperModel::REVISION
        ))
    );
    assert_eq!(WhisperModel::language_name("yue"), "Cantonese");
    assert_eq!(WhisperModel::language_name("xx"), "xx");
    let unique: HashSet<_> = WhisperModel::LANGUAGES.iter().collect();
    assert_eq!(unique.len(), 100);
}
