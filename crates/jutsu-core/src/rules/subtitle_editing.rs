//! The subtitle editor's changes to a version's cues.

use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::models::{Subtitles, WordSpacing};
use crate::rules::{CueBuilder, SubtitleCue};
use crate::support::Seconds;
use crate::support::text::char_count;

/// The subtitle editor's changes to a version's cues: text, merging a cue with the next, and splitting one in two.
/// Times stay within the cues', so cues never overlap. In a language written without spaces, a cue splits between any
/// two characters.
impl Subtitles {
    /// Sets the text of the cue `id`.
    pub fn set_text(&mut self, text: &str, id: Uuid) {
        if let Some(cue) = self.cues.iter_mut().find(|cue| cue.id == id) {
            text.clone_into(&mut cue.text);
        }
    }

    /// Whether the cue and the next one fit in one cue: two lines, at most `CueBuilder::MAX_DURATION`.
    pub fn can_merge(&self, id: Uuid) -> bool {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return false;
        };
        let Some(next) = self.cues.get(index + 1) else {
            return false;
        };
        let words = [self.units(&self.cues[index]), self.units(next)].concat();
        next.end - self.cues[index].start <= CueBuilder::MAX_DURATION
            && CueBuilder::wrap(&words, self.separator()).is_some()
    }

    /// The cue and the next one as one, its words wrapped again.
    pub fn merge_with_next(&mut self, id: Uuid) {
        if !self.can_merge(id) {
            return;
        }
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let next = self.cues.remove(index + 1);
        let separator = self.separator();
        let words = [self.units(&self.cues[index]), self.units(&next)].concat();
        let lines = CueBuilder::wrap(&words, separator).unwrap_or_else(|| vec![words.join(separator)]);
        self.cues[index].end = next.end;
        self.cues[index].text = lines.join("\n");
    }

    /// Whether the cue has more than one word.
    pub fn can_split(&self, id: Uuid) -> bool {
        self.cues
            .iter()
            .find(|cue| cue.id == id)
            .is_some_and(|cue| self.units(cue).len() > 1)
    }

    /// The cue as two at the word that halves its text best, its time shared by their lengths.
    pub fn split(&mut self, id: Uuid) {
        if !self.can_split(id) {
            return;
        }
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let cue = self.cues[index].clone();
        let words = self.units(&cue);
        let separator = self.separator();
        let length = |part: &[String]| char_count(&part.join(separator));
        let imbalance = |split: usize| length(&words[..split]).abs_diff(length(&words[split..]));
        let mut split = 1;
        for candidate in 2..words.len() {
            if imbalance(candidate) < imbalance(split) {
                split = candidate;
            }
        }
        let (first, second) = (words[..split].to_vec(), words[split..].to_vec());
        let total = (length(&first) + length(&second)).max(1);
        let share = length(&first) as f64 / total as f64;
        let middle: Seconds = cue.start + (cue.end - cue.start) * share;
        let first_text = CueBuilder::wrap(&first, separator).unwrap_or(first).join("\n");
        let second_text = CueBuilder::wrap(&second, separator).unwrap_or(second).join("\n");
        self.cues[index] = SubtitleCue {
            id: cue.id,
            start: cue.start,
            end: middle,
            text: first_text,
        };
        self.cues
            .insert(index + 1, SubtitleCue::new(middle, cue.end, second_text));
    }

    /// The cue showing at `time`, if any.
    pub fn cue_at(&self, time: Seconds) -> Option<&SubtitleCue> {
        self.cues.iter().find(|cue| cue.start <= time && time < cue.end)
    }

    fn separator(&self) -> &'static str {
        WordSpacing::separator(self.language())
    }

    /// The cue's words; each character in a language written without spaces.
    fn units(&self, cue: &SubtitleCue) -> Vec<String> {
        let parts = cue
            .text
            .split(char::is_whitespace)
            .filter(|part| !part.is_empty());
        if self.separator().is_empty() {
            parts
                .collect::<String>()
                .graphemes(true)
                .map(str::to_owned)
                .collect()
        } else {
            parts.map(str::to_owned).collect()
        }
    }
}
