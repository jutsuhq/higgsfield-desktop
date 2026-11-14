//! Timed words into subtitle cues.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::TimedWord;
use crate::support::Seconds;
use crate::support::text::{char_count, is_newline, trim, trim_whitespace};

/// One subtitle on screen: up to two lines from `start` to `end`, in seconds from the start of the video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SubtitleCue {
    /// Identifies the cue.
    pub id: Uuid,
    /// When it shows.
    pub start: Seconds,
    /// When it goes.
    pub end: Seconds,
    /// Its lines, joined by "\n".
    pub text: String,
}

impl SubtitleCue {
    /// A cue with a new ID.
    pub fn new(start: Seconds, end: Seconds, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            start,
            end,
            text: text.into(),
        }
    }

    /// The lines that show, without empty ones.
    pub fn lines(&self) -> Vec<String> {
        self.text
            .split(is_newline)
            .map(|line| trim_whitespace(line).to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }
}

/// Timed words into subtitle cues by the usual rules: at most two lines of 42 characters, each cue on screen 1 to 7
/// seconds, split at sentence ends and pauses, never overlapping.
///
/// A cue closes when the next word wouldn't fit in two lines or would keep it over 7 seconds, after a pause of half a
/// second once it has shown for a second (or after a pause of 1.5 seconds at any time), and at a sentence end once it
/// holds half a line. A word whose end runs on into a pause is cut back first, so the pause counts. A cue of one word
/// after a long pause joins a neighbour where both fit in one cue. Then short cues are stretched to a second where the
/// next one leaves room, and no cue runs into the next. Words are joined by the language's separator
/// (`WordSpacing::separator`): a space, or nothing in a language written without spaces.
pub struct CueBuilder;

impl CueBuilder {
    /// Characters on one line.
    pub const MAX_LINE_LENGTH: usize = 42;
    /// The shortest a cue shows, where the next one leaves room.
    pub const MIN_DURATION: Seconds = 1.0;
    /// The longest a cue shows.
    pub const MAX_DURATION: Seconds = 7.0;
    /// A pause that closes a cue once it has shown for `MIN_DURATION`.
    pub const PAUSE: Seconds = 0.5;
    /// A pause this long closes a cue that has shown for less than `MIN_DURATION`, and makes a lone word after it a
    /// candidate to join a neighbour.
    pub const LONG_PAUSE: Seconds = 1.5;
    const SENTENCE_ENDS: [char; 7] = ['.', '?', '!', '…', '。', '？', '！'];

    /// The cues for `words`, joined by `separator` (" " in Swift's default).
    pub fn cues(words: &[TimedWord], separator: &str) -> Vec<SubtitleCue> {
        let words: Vec<TimedWord> = Self::capped(
            words
                .iter()
                .filter(|word| !trim(&word.text).is_empty())
                .cloned()
                .collect(),
        );
        let mut groups: Vec<Vec<TimedWord>> = Vec::new();
        let mut current: Vec<TimedWord> = Vec::new();
        for word in words {
            if let (Some(first), Some(last)) = (current.first(), current.last())
                && Self::closes(&current, first, last, &word, separator)
            {
                groups.push(std::mem::take(&mut current));
            }
            current.push(word);
        }
        if !current.is_empty() {
            groups.push(current);
        }
        Self::timed(&Self::joining_lone_words(groups, separator), separator)
    }

    /// The longest a word is taken to last: about a syllable's time per two characters.
    pub fn longest_duration(word: &str) -> Seconds {
        0.3 + 0.08 * char_count(word) as f64
    }

    /// Ends cut back to `longest_duration`: a model often stretches a word over the pause after it.
    pub fn capped(words: Vec<TimedWord>) -> Vec<TimedWord> {
        words
            .into_iter()
            .map(|mut word| {
                word.end = word
                    .start
                    .max(word.end.min(word.start + Self::longest_duration(&word.text)));
                word
            })
            .collect()
    }

    fn texts(words: &[TimedWord]) -> Vec<String> {
        words.iter().map(|word| word.text.clone()).collect()
    }

    fn closes(
        current: &[TimedWord],
        first: &TimedWord,
        last: &TimedWord,
        word: &TimedWord,
        separator: &str,
    ) -> bool {
        let gap = word.start - last.end;
        let too_long = word.end - first.start > Self::MAX_DURATION;
        let mut with_word = Self::texts(current);
        with_word.push(word.text.clone());
        let no_fit = Self::wrap(&with_word, separator).is_none();
        let sentence = last
            .text
            .chars()
            .next_back()
            .is_some_and(|c| Self::SENTENCE_ENDS.contains(&c))
            && char_count(&Self::texts(current).join(separator)) >= Self::MAX_LINE_LENGTH / 2;
        let paused =
            gap >= Self::PAUSE && (last.end - first.start >= Self::MIN_DURATION || gap >= Self::LONG_PAUSE);
        paused || too_long || no_fit || sentence
    }

    /// A one-word cue after a long pause joins the next cue, or else the one before, when the two fit in one.
    fn joining_lone_words(mut groups: Vec<Vec<TimedWord>>, separator: &str) -> Vec<Vec<TimedWord>> {
        let mut index = 0;
        while index < groups.len() {
            let lone = groups[index].clone();
            let previous_end = if index > 0 {
                groups[index - 1].last().map_or(0.0, |word| word.end)
            } else {
                0.0
            };
            let after_pause = lone[0].start - previous_end >= Self::LONG_PAUSE;
            if lone.len() != 1 || !after_pause || groups.len() <= 1 {
                index += 1;
                continue;
            }
            let with_next = (index + 1 < groups.len())
                .then(|| [lone.as_slice(), groups[index + 1].as_slice()].concat())
                .filter(|joined| Self::fits(joined, separator));
            let with_previous = || {
                (index > 0)
                    .then(|| [groups[index - 1].as_slice(), lone.as_slice()].concat())
                    .filter(|joined| Self::fits(joined, separator))
            };
            if let Some(joined) = with_next {
                groups[index + 1] = joined;
                groups.remove(index);
            } else if let Some(joined) = with_previous() {
                groups[index - 1] = joined;
                groups.remove(index);
            } else {
                index += 1;
            }
        }
        groups
    }

    fn fits(words: &[TimedWord], separator: &str) -> bool {
        let (Some(first), Some(last)) = (words.first(), words.last()) else {
            return true;
        };
        last.end - first.start <= Self::MAX_DURATION && Self::wrap(&Self::texts(words), separator).is_some()
    }

    fn timed(groups: &[Vec<TimedWord>], separator: &str) -> Vec<SubtitleCue> {
        let mut cues: Vec<SubtitleCue> = groups
            .iter()
            .filter_map(|group| {
                let (first, last) = (group.first()?, group.last()?);
                let words = Self::texts(group);
                let lines = Self::wrap(&words, separator).unwrap_or_else(|| vec![words.join(separator)]);
                Some(SubtitleCue::new(first.start, last.end, lines.join("\n")))
            })
            .collect();
        for index in 0..cues.len() {
            let next = if index + 1 < cues.len() {
                cues[index + 1].start
            } else {
                cues[index].end + Self::MIN_DURATION
            };
            if cues[index].end - cues[index].start < Self::MIN_DURATION {
                cues[index].end = (cues[index].start + Self::MIN_DURATION).min(next);
            }
            cues[index].end = cues[index].start.max(cues[index].end.min(next));
        }
        cues
    }

    /// The words on one line, or on two of about the same length; `None` when they don't fit. A single word too long
    /// for a line keeps a line of its own.
    pub fn wrap(words: &[String], separator: &str) -> Option<Vec<String>> {
        let text = words.join(separator);
        if char_count(&text) <= Self::MAX_LINE_LENGTH || words.len() == 1 {
            return Some(vec![text]);
        }
        let mut best: Option<(usize, Vec<String>)> = None;
        for split in 1..words.len() {
            let first = words[..split].join(separator);
            let second = words[split..].join(separator);
            let (first_count, second_count) = (char_count(&first), char_count(&second));
            if first_count > Self::MAX_LINE_LENGTH || second_count > Self::MAX_LINE_LENGTH {
                continue;
            }
            let score = first_count.abs_diff(second_count);
            if best.as_ref().is_none_or(|(best_score, _)| score < *best_score) {
                best = Some((score, vec![first, second]));
            }
        }
        best.map(|(_, lines)| lines)
    }
}
