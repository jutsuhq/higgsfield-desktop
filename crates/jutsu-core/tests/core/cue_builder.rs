//! `CueBuilderTests.swift`.

use jutsu_core::*;
use time::OffsetDateTime;
use uuid::Uuid;

/// Words of `text` spoken one after another from `start`, each `step` long with no gap.
fn spoken(text: &str, start: f64, step: f64) -> Vec<TimedWord> {
    text.split(' ')
        .enumerate()
        .map(|(offset, word)| {
            TimedWord::new(
                word,
                start + offset as f64 * step,
                start + (offset + 1) as f64 * step,
            )
        })
        .collect()
}

fn said(text: &str) -> Vec<TimedWord> {
    spoken(text, 0.0, 0.3)
}

fn cues(words: &[TimedWord]) -> Vec<SubtitleCue> {
    CueBuilder::cues(words, " ")
}

fn lines(cues: &[SubtitleCue]) -> Vec<Vec<String>> {
    cues.iter().map(SubtitleCue::lines).collect()
}

fn texts(lines: &[&[&str]]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|cue| cue.iter().map(|line| (*line).to_owned()).collect())
        .collect()
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn subtitles(transcript: Transcript, cues: Option<Vec<SubtitleCue>>) -> Subtitles {
    Subtitles::new(transcript, cues, OffsetDateTime::now_utc())
}

fn transcript(language: &str, words: Vec<TimedWord>) -> Transcript {
    Transcript {
        language: language.into(),
        model: SpeechModel::Transcriber,
        revision: "1".into(),
        words,
    }
}

#[test]
fn a_long_stream_keeps_every_rule() {
    let sentence = "the quick brown fox jumps over the lazy dog while the band plays on and on";
    let words = spoken(&[sentence; 12].join(" "), 0.0, 0.25);
    let cues = cues(&words);

    assert!(cues.len() > 10);
    for cue in &cues {
        assert!(cue.lines().len() <= 2);
        assert!(
            cue.lines()
                .iter()
                .all(|line| line.chars().count() <= CueBuilder::MAX_LINE_LENGTH)
        );
        assert!(cue.end - cue.start <= CueBuilder::MAX_DURATION);
        assert!(cue.end - cue.start >= CueBuilder::MIN_DURATION - 1e-9);
    }
    for pair in cues.windows(2) {
        assert!(pair[0].end <= pair[1].start);
    }
    let joined = cues
        .iter()
        .flat_map(SubtitleCue::lines)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(joined.split(' ').count(), words.len());
}

#[test]
fn a_sentence_end_closes_a_cue_that_holds_half_a_line() {
    let cues = cues(&said(
        "This sentence is long enough here. And then another one starts.",
    ));
    assert_eq!(
        lines(&cues),
        texts(&[
            &["This sentence is long enough here."],
            &["And then another one starts."]
        ])
    );

    let short = self::cues(&said("Hi there. How are you doing today?"));
    assert_eq!(lines(&short), texts(&[&["Hi there. How are you doing today?"]]));
}

#[test]
fn a_pause_closes_a_cue_once_it_has_shown_for_a_second() {
    let cues = cues(&[said("one two three four"), spoken("five six", 1.9, 0.3)].concat());
    assert_eq!(lines(&cues), texts(&[&["one two three four"], &["five six"]]));

    // Shown for 0.6 s: a pause under 1.5 s doesn't close it.
    let early = self::cues(&[said("one two"), spoken("three four", 1.4, 0.3)].concat());
    assert_eq!(lines(&early), texts(&[&["one two three four"]]));
}

#[test]
fn a_word_that_runs_on_into_a_pause_is_cut_back() {
    // "four" claims to last until 3.4 s; cut back, the 1.9 s pause before "five" closes the cue there.
    let mut words = said("one two three four");
    words[3].end = 3.4;
    let cues = cues(&[words, spoken("five six", 3.5, 0.3)].concat());

    assert_eq!(lines(&cues), texts(&[&["one two three four"], &["five six"]]));
    assert!(cues[0].end < 2.0);
}

#[test]
fn a_lone_word_after_a_long_pause_joins_the_next_cue() {
    // "Absolutely" shows for over a second, so the pause after it closes its cue; alone, it joins the next.
    let words = [
        said("This first sentence is long enough."),
        vec![TimedWord::new("Absolutely", 6.0, 7.1)],
        spoken("we start again", 7.7, 0.3),
    ]
    .concat();
    assert_eq!(
        lines(&cues(&words)),
        texts(&[
            &["This first sentence is long enough."],
            &["Absolutely we start again"]
        ])
    );
}

#[test]
fn a_lone_word_joins_the_cue_before_when_the_next_is_full() {
    let full = "this next cue is already filled to the brim with words and more words on it";
    let words = [
        said("Short start"),
        vec![TimedWord::new("well", 2.5, 2.8)],
        spoken(full, 4.4, 0.4),
    ]
    .concat();
    let cues = cues(&words);
    assert_eq!(
        cues.first().map(SubtitleCue::lines),
        Some(strings(&["Short start well"]))
    );
}

#[test]
#[allow(clippy::float_cmp, reason = "the times are exact, as the Swift test expects")]
fn short_cues_stretch_to_a_second_without_running_into_the_next() {
    let cues = cues(
        &[
            spoken("This is a long first sentence.", 0.0, 0.1),
            spoken("And another thing here.", 0.8, 0.3),
        ]
        .concat(),
    );
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].end, 0.8);

    let last = self::cues(&[TimedWord::new("Bye.", 5.0, 5.2)]);
    assert_eq!(last.first().map(|cue| cue.end - cue.start), Some(1.0));
}

#[test]
fn wrapping_balances_two_lines_and_refuses_three() {
    let words: Vec<String> = "a quick brown fox jumps over the lazy dog and keeps on running far"
        .split(' ')
        .map(str::to_owned)
        .collect();
    let lines = CueBuilder::wrap(&words, " ").unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].chars().count().abs_diff(lines[1].chars().count()) <= 6);
    assert_eq!(CueBuilder::wrap(&vec!["wordy".to_owned(); 20], " "), None);
    let long = "x".repeat(50);
    assert_eq!(
        CueBuilder::wrap(std::slice::from_ref(&long), " "),
        Some(vec![long])
    );
}

#[test]
fn subtitle_files_are_timed_to_the_millisecond() {
    let cues = [
        SubtitleCue::new(1.2, 3.4567, "Hello there,\nfriend."),
        SubtitleCue::new(3.5, 3.9, "  \n"),
        SubtitleCue::new(3661.0004, 3662.0, "A --> B"),
    ];
    assert_eq!(
        SubtitleFormat::Srt.text(&cues),
        "1\n00:00:01,200 --> 00:00:03,457\nHello there,\nfriend.\n\n2\n01:01:01,000 --> 01:01:02,000\nA → B\n"
    );
    assert_eq!(
        SubtitleFormat::Vtt.text(&cues),
        "WEBVTT\n\n00:00:01.200 --> 00:00:03.457\nHello there,\nfriend.\n\n01:01:01.000 --> 01:01:02.000\nA → B\n"
    );
}

#[test]
#[allow(clippy::float_cmp, reason = "the times are exact, as the Swift test expects")]
fn editing_merges_splits_and_finds_the_cue_on_screen() {
    let a = SubtitleCue::new(0.0, 2.0, "Hello there");
    let b = SubtitleCue::new(2.0, 4.0, "my good friend");
    let mut subtitles = subtitles(transcript("en-US", Vec::new()), Some(vec![a.clone(), b.clone()]));

    assert_eq!(subtitles.cue_at(2.5).map(|cue| cue.id), Some(b.id));
    assert!(subtitles.cue_at(5.0).is_none());
    assert!(subtitles.can_merge(a.id));
    assert!(!subtitles.can_merge(b.id));
    subtitles.merge_with_next(a.id);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>(),
        ["Hello there my good friend"]
    );
    assert_eq!(subtitles.cues[0].end, 4.0);

    subtitles.split(a.id);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>(),
        ["Hello there", "my good friend"]
    );
    assert_eq!(subtitles.cues[0].end, subtitles.cues[1].start);
    assert!((subtitles.cues[0].end - 4.0 * 11.0 / 25.0).abs() < 1e-9);

    subtitles.set_text("Hi there", a.id);
    assert_eq!(subtitles.cues[0].text, "Hi there");
    let one: Uuid = subtitles.cues.last().unwrap().id;
    subtitles.set_text("friend", one);
    assert!(!subtitles.can_split(one));
}

#[test]
fn subtitles_build_their_cues_from_the_transcript() {
    let subtitles = subtitles(transcript("en-US", said("Hello there.")), None);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>(),
        ["Hello there."]
    );
}

/// Runs as a model writes them, each `step` long.
fn runs(texts: &[&str]) -> Vec<TimedWord> {
    texts
        .iter()
        .enumerate()
        .map(|(offset, text)| TimedWord::new(*text, offset as f64 * 0.3, (offset + 1) as f64 * 0.3))
        .collect()
}

#[test]
#[allow(clippy::float_cmp, reason = "the times are exact, as the Swift test expects")]
fn runs_without_a_leading_space_join_the_word_before_in_a_spaced_language() {
    let mut words = Vec::new();
    WordSpacing::append(
        &runs(&["Hello", ",", " world", "s", " again."]),
        &mut words,
        "en-US",
    );

    assert_eq!(
        words.iter().map(|word| word.text.as_str()).collect::<Vec<_>>(),
        ["Hello,", "worlds", "again."]
    );
    assert!((words[1].start - 0.6).abs() < 1e-12 && (words[1].end - 1.2).abs() < 1e-12);
}

#[test]
fn a_language_without_spaces_keeps_each_run_as_a_word_and_joins_with_nothing() {
    let mut words = Vec::new();
    let spoken = [
        "今日",
        "は",
        "とても",
        "いい",
        "天気",
        "です",
        "ね",
        "。",
        "明日",
        "も",
        "晴れ",
        "る",
        "でしょう",
        "か",
        "？",
    ];
    WordSpacing::append(&runs(&spoken), &mut words, "ja-JP");

    assert_eq!(words.len(), spoken.len() - 2);
    assert_eq!(words[6].text, "ね。");
    let transcript = transcript("ja-JP", words);
    assert_eq!(
        transcript.text(),
        "今日はとてもいい天気ですね。明日も晴れるでしょうか？"
    );
    let mut subtitles = subtitles(transcript.clone(), None);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<Vec<_>>(),
        ["今日はとてもいい天気ですね。明日も晴れるでしょうか？"]
    );

    let cue = subtitles.cues[0].id;
    assert!(subtitles.can_split(cue));
    subtitles.split(cue);
    assert_eq!(subtitles.cues.len(), 2);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.as_str())
            .collect::<String>(),
        transcript.text()
    );
    assert!(subtitles.cues.iter().all(|cue| !cue.text.contains(' ')));
    let first = subtitles.cues[0].id;
    subtitles.merge_with_next(first);
    assert_eq!(
        subtitles
            .cues
            .iter()
            .map(|cue| cue.text.clone())
            .collect::<Vec<_>>(),
        [transcript.text()]
    );
    assert!(WordSpacing::separator("zh-Hans-CN").is_empty() && WordSpacing::separator("yue").is_empty());
    assert_eq!(WordSpacing::separator("de-DE"), " ");
}

mod delivery_subtitles {
    use jutsu_core::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn set(outputs: Vec<DeliveryOutput>) -> DeliverySet {
        let reference = MaterialReference {
            material_id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
        };
        let item = DeliveryItem::new(reference, MaterialKind::Video, "clip");
        DeliverySet::new(
            Uuid::new_v4(),
            "Set",
            OffsetDateTime::now_utc(),
            vec![item],
            outputs,
        )
    }

    fn output(subtitles: Option<DeliverySubtitles>) -> DeliveryOutput {
        let mut output = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
        output.subtitles = subtitles;
        output
    }

    #[test]
    fn subtitle_files_need_free_names_beside_the_video() {
        let output = output(Some(DeliverySubtitles::Files));
        let set = set(vec![output.clone()]);
        let free = DeliveryPlan::tasks(&set, DeliveryScope::All, |_| false);
        let base = free[0].file_name.rsplit_once('.').unwrap().0.to_owned();

        let vtt = format!("{base}.vtt");
        let tasks = DeliveryPlan::tasks(&set, DeliveryScope::All, |name| name == vtt);
        assert_eq!(tasks.len(), 1);
        assert_ne!(tasks[0].file_name, free[0].file_name);
        assert_eq!(DeliveryPlan::names(&tasks[0].file_name, &output).len(), 3);
        assert_eq!(DeliveryPlan::names("a.mp4", &self::output(None)), ["a.mp4"]);
    }

    #[test]
    fn burning_in_names_the_output_and_old_outputs_still_decode() {
        let burned = output(Some(DeliverySubtitles::BurnIn));
        assert_eq!(burned.summary(), "editingH264-subtitled");
        assert!(burned.subtitle_formats().is_empty());
        let mut json = serde_json::to_value(&burned).unwrap();
        json.as_object_mut().unwrap().remove("subtitles");
        let old: DeliveryOutput = serde_json::from_value(json).unwrap();
        assert_eq!(old.subtitles, None);
    }
}
