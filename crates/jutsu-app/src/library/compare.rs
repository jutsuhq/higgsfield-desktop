//! Comparing results (`Compare.swift`): two library results on one clock, the side-by-side grid of a batch, and
//! what "Compare with…" offers. The clock itself runs in the window, where the videos play.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::item::LibraryItem;

/// How the compare surface shows its videos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CompareLayout {
    SideBySide,
    Divider,
}

impl CompareLayout {
    pub fn title(self) -> &'static str {
        match self {
            Self::SideBySide => "Side by side",
            Self::Divider => "Divider",
        }
    }

    /// The divider lays one video over another, so it takes exactly two.
    pub fn available(count: usize) -> Vec<Self> {
        if count == 2 {
            vec![Self::SideBySide, Self::Divider]
        } else {
            vec![Self::SideBySide]
        }
    }

    /// Columns of the side-by-side grid: one row up to three videos, then as square as it gets.
    pub fn columns(count: usize) -> usize {
        if count <= 3 {
            return count.max(1);
        }
        (1..=count)
            .find(|columns| columns * columns >= count)
            .unwrap_or(count)
    }
}

/// The chips put before two versions of one item.
pub const ORIGINAL_LABEL: &str = "Original";
pub const EDITED_LABEL: &str = "New version";

/// Two library results compared on one page: which plays on the left, which on the right.
#[derive(Debug, Clone, PartialEq)]
pub struct ComparePair {
    pub left: LibraryItem,
    pub right: LibraryItem,
    /// Chips put before a side's own, by item: "Original" and "New version" for two versions of one item.
    pub labels: HashMap<Uuid, Vec<String>>,
}

impl ComparePair {
    /// `None` for the same item twice or a result without a local file.
    pub fn new(left: LibraryItem, right: LibraryItem) -> Option<Self> {
        (left.id() != right.id() && left.output_file().is_some() && right.output_file().is_some()).then(
            || Self {
                left,
                right,
                labels: HashMap::new(),
            },
        )
    }

    /// Two versions of one item: the one it was made from on the left.
    pub fn versions(original: LibraryItem, edited: LibraryItem) -> Option<Self> {
        let mut pair = Self::new(original, edited)?;
        pair.labels = [
            (pair.left.id(), vec![ORIGINAL_LABEL.to_owned()]),
            (pair.right.id(), vec![EDITED_LABEL.to_owned()]),
        ]
        .into();
        Some(pair)
    }

    /// Left first: each side with its labels, the pair's chips before the settings chips.
    pub fn tracks(&self) -> Vec<(&LibraryItem, Vec<String>)> {
        [&self.left, &self.right]
            .into_iter()
            .filter(|item| item.output_file().is_some())
            .map(|item| {
                let mut labels = self.labels.get(&item.id()).cloned().unwrap_or_default();
                labels.extend(item.facts());
                (item, labels)
            })
            .collect()
    }
}

/// What "Compare with…" offers for `item`: other results on disk, those made from the same source first, otherwise
/// in the library's order. At most `limit`, so the menu stays short.
pub fn candidates<'a>(item: &LibraryItem, items: &'a [LibraryItem], limit: usize) -> Vec<&'a LibraryItem> {
    let source = item.spec().source.as_ref().map(|clip| &clip.file_url);
    let others = items
        .iter()
        .filter(|other| other.id() != item.id() && other.output_file().is_some());
    let same = |other: &&LibraryItem| {
        source.is_some() && other.spec().source.as_ref().map(|clip| &clip.file_url) == source
    };
    others
        .clone()
        .filter(same)
        .chain(others.filter(|other| !same(other)))
        .take(limit)
        .collect()
}

/// "rooftop · Claymation — 480p draft", a "Compare with…" menu item.
pub fn menu_title(item: &LibraryItem) -> String {
    let kind = if item.spec().is_draft { "draft" } else { "final" };
    format!("{} — {} {kind}", item.title(), item.resolution_title())
}

/// Picking two results in the Library grid to compare (`ComparePicker`): a third pick replaces the oldest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ComparePick {
    /// The picks in the order they were picked.
    pub picked: Vec<String>,
    /// The pick bar's text: "Pick two results to compare", "Pick one more", "Ready to compare".
    pub prompt: String,
    /// Compare is enabled: two are picked.
    pub is_ready: bool,
}

impl ComparePick {
    /// `picked` after `toggled` is picked, or unpicked when it already was.
    pub fn new(mut picked: Vec<String>, toggled: Option<String>) -> Self {
        if let Some(id) = toggled {
            if let Some(index) = picked.iter().position(|pick| *pick == id) {
                picked.remove(index);
            } else {
                picked.push(id);
            }
        }
        let extra = picked.len().saturating_sub(2);
        picked.drain(..extra);
        let prompt = match picked.len() {
            0 => "Pick two results to compare",
            1 => "Pick one more",
            _ => "Ready to compare",
        };
        Self {
            is_ready: picked.len() == 2,
            prompt: prompt.to_owned(),
            picked,
        }
    }
}

#[cfg(test)]
mod tests {
    use jutsu_core::Resolution;

    use super::*;
    use crate::library::fixture::ItemFixture;

    /// `ComparePickerTests`: a third pick replaces the oldest; picking again unpicks.
    #[test]
    fn the_picker_keeps_the_last_two_picks() {
        let pick = |picked: &[&str], toggled: &str| {
            ComparePick::new(
                picked.iter().map(|id| (*id).to_owned()).collect(),
                Some(toggled.to_owned()),
            )
        };
        let empty = ComparePick::new(Vec::new(), None);
        assert_eq!(
            (empty.prompt.as_str(), empty.is_ready),
            ("Pick two results to compare", false)
        );
        assert_eq!(pick(&[], "a").prompt, "Pick one more");
        let two = pick(&["a"], "b");
        assert_eq!((two.prompt.as_str(), two.is_ready), ("Ready to compare", true));
        assert_eq!(pick(&["a", "b"], "c").picked, ["b", "c"]);
        assert_eq!(pick(&["a", "b"], "a").picked, ["b"]);
    }

    fn at(resolution: Resolution, is_draft: bool, preset_name: Option<&'static str>) -> LibraryItem {
        ItemFixture {
            resolution,
            is_draft,
            preset_name,
            ..ItemFixture::default()
        }
        .item()
    }

    fn without_file(mut item: LibraryItem) -> LibraryItem {
        item.job.output_file = None;
        item
    }

    fn from(source: &'static str, resolution: Resolution, is_draft: bool) -> LibraryItem {
        ItemFixture {
            source,
            resolution,
            is_draft,
            ..ItemFixture::default()
        }
        .item()
    }

    /// `ComparePairTests.needsTwoDifferentResultsOnDisk`.
    #[test]
    fn needs_two_different_results_on_disk() {
        let draft = at(Resolution::P480, true, None);
        let final_ = at(Resolution::P720, false, None);
        assert!(ComparePair::new(draft.clone(), final_.clone()).is_some());
        assert!(ComparePair::new(draft.clone(), draft.clone()).is_none());
        assert!(ComparePair::new(draft, without_file(final_)).is_none());
    }

    /// `ComparePairTests.tracksAreLabelledWithTheSettingsChipsLeftFirst`.
    #[test]
    fn tracks_are_labelled_with_the_settings_chips_left_first() {
        let draft = at(Resolution::P480, true, Some("Claymation"));
        let final_ = at(Resolution::P1080, false, Some("Film Noir"));
        let pair = ComparePair::new(draft.clone(), final_.clone()).unwrap();
        let tracks = pair.tracks();
        assert_eq!(
            tracks.iter().map(|(item, _)| item.id()).collect::<Vec<_>>(),
            [draft.id(), final_.id()]
        );
        assert_eq!(tracks[0].1, draft.facts());
        assert_eq!(tracks[1].1, final_.facts());
        assert_eq!(tracks[0].1, ["Restyle", "Claymation", "480p", "6 s"]);
    }

    /// Two versions of one item are labelled "Original" and "New version" first.
    #[test]
    fn versions_are_labelled_original_and_new() {
        let original = at(Resolution::P480, true, None);
        let edited = at(Resolution::P480, true, None);
        let pair = ComparePair::versions(original, edited).unwrap();
        let tracks = pair.tracks();
        assert_eq!(tracks[0].1.first().map(String::as_str), Some("Original"));
        assert_eq!(tracks[1].1.first().map(String::as_str), Some("New version"));
    }

    /// `ComparePairTests.candidatesPutTheSameSourceFirstAndSkipTheItemAndMissingFiles`.
    #[test]
    fn candidates_put_the_same_source_first_and_skip_the_item_and_missing_files() {
        let item = from("rooftop.mov", Resolution::P480, true);
        let other = from("dance.mp4", Resolution::P480, true);
        let sibling = from("rooftop.mov", Resolution::P720, false);
        let missing = without_file(from("rooftop.mov", Resolution::P480, true));
        let library = [other.clone(), item.clone(), missing, sibling.clone()];
        let ids = |found: Vec<&LibraryItem>| found.iter().map(|item| item.id()).collect::<Vec<_>>();
        assert_eq!(ids(candidates(&item, &library, 12)), [sibling.id(), other.id()]);
        assert_eq!(ids(candidates(&item, &library, 1)), [sibling.id()]);
        assert_eq!(candidates(&item, std::slice::from_ref(&item), 12).len(), 0);
    }

    /// `ComparePairTests.menuTitleNamesTheResolutionAndKind`.
    #[test]
    fn menu_title_names_the_resolution_and_kind() {
        let draft = at(Resolution::P480, true, Some("Claymation"));
        let final_ = at(Resolution::P1080, false, Some("Claymation"));
        assert_eq!(menu_title(&draft), "rooftop · Claymation — 480p draft");
        assert_eq!(menu_title(&final_), "rooftop · Claymation — 1080p final");
    }

    /// `CompareLayoutTests.dividerOnlyForTwo`.
    #[test]
    fn divider_only_for_two() {
        assert_eq!(
            CompareLayout::available(2),
            [CompareLayout::SideBySide, CompareLayout::Divider]
        );
        assert_eq!(CompareLayout::available(1), [CompareLayout::SideBySide]);
        assert_eq!(CompareLayout::available(4), [CompareLayout::SideBySide]);
    }

    /// `CompareLayoutTests.oneRowUpToThreeThenSquare`.
    #[test]
    fn one_row_up_to_three_then_square() {
        let columns: Vec<usize> = [0, 2, 3, 4, 6, 9].map(CompareLayout::columns).to_vec();
        assert_eq!(columns, [1, 2, 3, 2, 3, 3]);
    }
}
