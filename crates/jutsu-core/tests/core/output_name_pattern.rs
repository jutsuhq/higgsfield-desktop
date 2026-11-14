//! `OutputNamePatternTests.swift`.

use std::collections::HashSet;

use jutsu_core::*;
use time::UtcOffset;
use time::macros::datetime;
use uuid::Uuid;

use crate::common::*;

const UTC: UtcOffset = UtcOffset::UTC;

fn job(mode: Mode, preset: bool, is_draft: bool) -> Job {
    let id = Uuid::parse_str("1A2B3C4D-0000-0000-0000-000000000000").unwrap();
    let mut spec = spec_with(SpecArgs {
        mode,
        preset_id: preset.then(Uuid::new_v4),
        resolution: Resolution::P720,
        ..SpecArgs::default()
    });
    spec.is_draft = is_draft;
    Job::new(
        id,
        spec,
        ProviderKind::ApiKey,
        usd("1", false),
        Uuid::new_v4(),
        datetime!(2026-03-07 23:30:05 UTC),
    )
}

fn swap() -> Job {
    job(Mode::ObjectSwap, false, false)
}

fn name(pattern: &str, job: &Job, preset_name: Option<&str>) -> String {
    OutputNamePattern::new(pattern).file_name(job, preset_name, UTC, None)
}

/// Swift names the time zone (Asia/Tokyo); a fixed offset stands in for it here.
#[test]
fn default_is_local_date_mode_and_short_id() {
    let job = swap();
    let tokyo = UtcOffset::from_hms(9, 0, 0).unwrap();
    assert_eq!(
        OutputNamePattern::standard().file_name(&job, None, UTC, None),
        "2026-03-07_objectSwap_1a2b3c4d.mp4"
    );
    assert_eq!(
        OutputNamePattern::standard().file_name(&job, None, tokyo, None),
        "2026-03-08_objectSwap_1a2b3c4d.mp4"
    );
}

#[test]
fn every_token_expands() {
    let name = name(
        "{date} {time} {mode} {preset} {res} {draft} {id}",
        &job(Mode::Restyle, true, true),
        Some("Film Noir"),
    );
    assert_eq!(
        name,
        "2026-03-07 23-30-05 restyle Film Noir 720p draft 1a2b3c4d.mp4"
    );
}

#[test]
fn missing_preset_and_non_draft_are_empty() {
    assert_eq!(
        name("{mode}{preset}{draft}_{id}", &swap(), Some("Ignored")),
        "objectSwap_1a2b3c4d.mp4"
    );
}

#[test]
fn empty_tokens_leave_no_stray_separators() {
    for (pattern, expected) in [
        ("{date}_{draft}_{mode}", "2026-03-07_objectSwap"),
        ("{draft} {preset} {mode}", "objectSwap"),
        ("{mode}-{draft}-{preset}-{id}", "objectSwap-1a2b3c4d"),
        ("{draft}_{mode}_{preset}", "objectSwap"),
        ("{mode}_{draft}-{id}", "objectSwap_-1a2b3c4d"),
        ("{mode}__{id}", "objectSwap__1a2b3c4d"),
    ] {
        assert_eq!(
            name(pattern, &swap(), None),
            format!("{expected}.mp4"),
            "{pattern}"
        );
    }
}

#[test]
fn unknown_preset_name_is_a_placeholder() {
    assert_eq!(
        name("{preset}_{id}", &job(Mode::ObjectSwap, true, false), None),
        "preset_1a2b3c4d.mp4"
    );
}

#[test]
fn invalid_pattern_falls_back_to_the_default() {
    for pattern in ["", "   ", "{nope}_{id}", "{date", "{Date}", "{{date}}"] {
        assert!(!OutputNamePattern::is_valid(pattern), "{pattern}");
        assert_eq!(
            OutputNamePattern::new(pattern).pattern(),
            OutputNamePattern::DEFAULT_PATTERN
        );
    }
}

#[test]
fn valid_pattern_is_kept() {
    for pattern in ["{date}", "clip", "a}b_{id}", OutputNamePattern::DEFAULT_PATTERN] {
        assert_eq!(OutputNamePattern::new(pattern).pattern(), pattern);
    }
}

#[test]
fn name_that_sanitizes_to_nothing_uses_the_default() {
    assert_eq!(
        name("..{draft}", &swap(), None),
        "2026-03-07_objectSwap_1a2b3c4d.mp4"
    );
}

#[test]
fn sanitizes() {
    for (input, expected) in [
        ("a/b:c", "a-b-c"),
        ("..hidden", "hidden"),
        ("  . x  ", "x"),
        ("line\nbreak\u{7}", "linebreak"),
        ("tab\there", "tabhere"),
        ("👩‍🎨 art", "👩‍🎨 art"),
    ] {
        assert_eq!(OutputNamePattern::sanitize(input), expected, "{input:?}");
    }
}

#[test]
fn long_names_are_cut_on_character_boundaries() {
    let cut = OutputNamePattern::sanitize(&"é".repeat(300));
    assert!(cut.len() <= OutputNamePattern::MAX_BASE_BYTES);
    assert_eq!(cut, "é".repeat(OutputNamePattern::MAX_BASE_BYTES / 2));
}

#[test]
fn preset_name_is_sanitized() {
    assert_eq!(
        name(
            "{preset}",
            &job(Mode::ObjectSwap, true, false),
            Some("Anime/Manga: v2")
        ),
        "Anime-Manga- v2.mp4"
    );
}

#[test]
fn taken_name_gets_the_first_free_counter() {
    let taken = HashSet::from(["clip.mp4", "clip-2.mp4", "clip-3.mp4"]);
    assert_eq!(
        OutputNamePattern::available("clip.mp4", |name| taken.contains(name)),
        "clip-4.mp4"
    );
    assert_eq!(OutputNamePattern::available("clip.mp4", |_| false), "clip.mp4");
}

#[test]
fn counter_goes_before_the_extension_only() {
    let taken = HashSet::from(["v1.0 final.mp4", "noext"]);
    assert_eq!(
        OutputNamePattern::available("v1.0 final.mp4", |name| taken.contains(name)),
        "v1.0 final-2.mp4"
    );
    assert_eq!(
        OutputNamePattern::available("noext", |name| taken.contains(name)),
        "noext-2"
    );
}
