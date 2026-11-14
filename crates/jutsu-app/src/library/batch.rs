//! The results of one batch, kept together in the grid and opened as a comparison (`LibraryBatch.swift`).

use std::collections::{HashMap, HashSet};

use jutsu_core::{Cost, JobSpec, Mode};
use time::OffsetDateTime;
use uuid::Uuid;

use super::item::LibraryItem;
use crate::queue::format;

/// One result in the comparison: the item and its label, the style's name or "Character 2".
#[derive(Debug, Clone, PartialEq)]
pub struct BatchCell {
    pub item: LibraryItem,
    pub label: String,
}

/// The results of one batch, or of one variant group, in the order they were queued.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryBatch {
    pub id: Uuid,
    pub items: Vec<LibraryItem>,
}

impl LibraryBatch {
    /// `items` in the order they were queued, which is the order the styles or characters were picked.
    pub fn new(id: Uuid, mut items: Vec<LibraryItem>) -> Self {
        items.sort_by(|left, right| {
            (left.job.created_at, left.id().to_string()).cmp(&(right.job.created_at, right.id().to_string()))
        });
        Self { id, items }
    }

    /// The batch, or the variant group, with this id among `items`; `None` when fewer than two of its results are
    /// left.
    pub fn find(id: Uuid, items: &[LibraryItem]) -> Option<Self> {
        let members: Vec<LibraryItem> = items
            .iter()
            .filter(|item| item.job.batch_id == Some(id) || item.job.variant_group_id == Some(id))
            .cloned()
            .collect();
        (members.len() > 1).then(|| Self::new(id, members))
    }

    /// The card's image.
    pub fn cover(&self) -> Option<&LibraryItem> {
        self.items.first()
    }

    pub fn mode(&self) -> Mode {
        self.items.first().map_or(Mode::Restyle, |item| item.spec().mode)
    }

    /// "dance · 4 styles".
    pub fn title(&self) -> String {
        let source = self
            .items
            .first()
            .map_or_else(|| "Batch".to_owned(), LibraryItem::source_name);
        let specs: Vec<&JobSpec> = self.items.iter().map(LibraryItem::spec).collect();
        format!("{source} · {}", variant_text(self.items.len(), &specs))
    }

    /// "Draft 480p · $7.63 · 36 min ago": the prices added up and the time of the last result.
    pub fn subtitle(&self, now: OffsetDateTime) -> String {
        let first = self.items.first();
        let resolution = first.map(LibraryItem::resolution_title).unwrap_or_default();
        let mut parts = vec![if first.is_some_and(|item| !item.spec().is_draft) {
            format!("Final {resolution}")
        } else {
            format!("Draft {resolution}")
        }];
        let prices: Vec<Cost> = self.items.iter().map(|item| item.job.price().clone()).collect();
        if let Some(total) = format::total(&prices) {
            parts.push(format::cost_text(&total));
        }
        let finished = self
            .items
            .iter()
            .map(LibraryItem::finished_at)
            .max()
            .unwrap_or(now);
        parts.push(format::relative_time(finished, now));
        parts.join(" · ")
    }

    pub fn is_starred(&self) -> bool {
        self.items.iter().any(|item| item.is_starred)
    }

    /// What "Delete Unstarred" deletes: the results nobody starred, which lost the comparison.
    pub fn unstarred(&self) -> Vec<&LibraryItem> {
        self.items.iter().filter(|item| !item.is_starred).collect()
    }

    /// What a screen reader reads for the card: "Batch: dance · 4 styles, Draft 480p · $7.63 · 36 min ago".
    pub fn accessibility_label(&self, now: OffsetDateTime) -> String {
        let mut parts = vec![format!("Batch: {}", self.title()), self.subtitle(now)];
        if self.is_starred() {
            parts.push("Has starred results".to_owned());
        }
        parts.join(", ")
    }

    /// The comparison's cells, labelled by style, by character number or by variant number.
    /// - `preset_names`: names of restyle presets the catalog knows.
    pub fn cells(&self, preset_names: &HashMap<Uuid, String>) -> Vec<BatchCell> {
        let specs: Vec<&JobSpec> = self.items.iter().map(LibraryItem::spec).collect();
        let variants = are_variants(&specs);
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let number = index + 1;
                let label = if variants {
                    format!("Variant {number}")
                } else if item.spec().mode == Mode::Restyle {
                    item.spec()
                        .preset_id
                        .and_then(|id| preset_names.get(&id).cloned())
                        .unwrap_or_else(|| format!("Style {number}"))
                } else {
                    format!("Character {number}")
                };
                BatchCell {
                    item: item.clone(),
                    label,
                }
            })
            .collect()
    }
}

/// One card of the grid's videos: a result, or a batch of results.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryEntry {
    Item(Box<LibraryItem>),
    Batch(LibraryBatch),
}

impl LibraryEntry {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Item(item) => item.id(),
            Self::Batch(batch) => batch.id,
        }
    }

    /// `items` in order, each batch with two or more results among them as one card at its first result.
    /// - `variants`: results of a variant group on screen; they stay cards of their own, since their group already
    ///   folds them.
    pub fn group(items: &[LibraryItem], variants: &HashSet<Uuid>) -> Vec<Self> {
        let batch_of = |item: &LibraryItem| item.job.batch_id.filter(|_| !variants.contains(&item.id()));
        let mut members: HashMap<Uuid, Vec<LibraryItem>> = HashMap::new();
        for item in items {
            if let Some(batch) = batch_of(item) {
                members.entry(batch).or_default().push(item.clone());
            }
        }
        let mut placed = HashSet::new();
        let mut entries = Vec::new();
        for item in items {
            match batch_of(item) {
                Some(batch) if placed.contains(&batch) => {}
                Some(batch) if members.get(&batch).is_some_and(|members| members.len() > 1) => {
                    placed.insert(batch);
                    let group = members.remove(&batch).unwrap_or_default();
                    entries.push(Self::Batch(LibraryBatch::new(batch, group)));
                }
                _ => entries.push(Self::Item(Box::new(item.clone()))),
            }
        }
        entries
    }
}

/// "4 styles" or "3 characters", after the mode the batch varies; "3 variants" for the jobs of one submit of
/// variants (`BatchGrouping.variantText`).
pub fn variant_text(count: usize, specs: &[&JobSpec]) -> String {
    if are_variants(specs) {
        return format!("{count} variants");
    }
    if specs.first().map_or(Mode::Restyle, |spec| spec.mode) == Mode::Restyle {
        format!("{count} styles")
    } else {
        format!("{count} characters")
    }
}

/// The jobs of one submit of variants share every input (`BatchGrouping.areVariants`).
pub fn are_variants(specs: &[&JobSpec]) -> bool {
    let Some(first) = specs.first() else {
        return false;
    };
    specs.len() > 1
        && first.mode != Mode::Restyle
        && specs.iter().all(|spec| {
            spec.references == first.references
                && spec.preset_id == first.preset_id
                && spec.source == first.source
                && spec.prompt == first.prompt
        })
}

#[cfg(test)]
mod tests {
    use jutsu_core::{Job, JobState, ProviderKind};

    use super::*;
    use crate::library::fixture::{ItemFixture, file, minutes_ago, now};

    /// Queued and finished `minutes_ago`, so an older member comes first in its batch.
    fn member(batch: Option<Uuid>, preset_name: Option<&'static str>, minutes: f64) -> LibraryItem {
        let base = ItemFixture {
            preset_name,
            minutes_ago: minutes,
            ..ItemFixture::default()
        }
        .item();
        let mut job = Job::new(
            Uuid::new_v4(),
            base.spec().clone(),
            ProviderKind::ApiKey,
            base.job.price().clone(),
            Uuid::new_v4(),
            minutes_ago(minutes + 1.0),
        );
        job.batch_id = batch;
        job.state = JobState::Done;
        job.finished_at = base.job.finished_at;
        job.output_file = base.job.output_file.clone();
        let mut item = LibraryItem::new(job, false);
        item.preset_name = preset_name.map(str::to_owned);
        item
    }

    /// `LibraryBatchTests.aBatchIsOneCardAtItsNewestResult`.
    #[test]
    fn a_batch_is_one_card_at_its_newest_result() {
        let batch = Uuid::new_v4();
        let newest = member(Some(batch), Some("Noir"), 1.0);
        let other = member(None, None, 2.0);
        let oldest = member(Some(batch), Some("Clay"), 3.0);
        let none = HashSet::new();
        let entries = LibraryEntry::group(&[newest.clone(), other.clone(), oldest], &none);
        assert_eq!(
            entries.iter().map(LibraryEntry::id).collect::<Vec<_>>(),
            [batch, other.id()]
        );
        let entries = LibraryEntry::group(&[newest.clone(), other.clone()], &none);
        assert_eq!(
            entries.iter().map(LibraryEntry::id).collect::<Vec<_>>(),
            [newest.id(), other.id()]
        );
    }

    /// A result of a variant group on screen stays a card of its own.
    #[test]
    fn variants_stay_cards_of_their_own() {
        let batch = Uuid::new_v4();
        let first = member(Some(batch), None, 1.0);
        let second = member(Some(batch), None, 2.0);
        let variants: HashSet<Uuid> = [first.id()].into();
        let entries = LibraryEntry::group(&[first.clone(), second.clone()], &variants);
        assert_eq!(
            entries.iter().map(LibraryEntry::id).collect::<Vec<_>>(),
            [first.id(), second.id()]
        );
    }

    /// `LibraryBatchTests.comparisonCellsAreLabeledByStyleOrCharacter`.
    #[test]
    fn comparison_cells_are_labeled_by_style_or_character() {
        let batch = Uuid::new_v4();
        let noir = member(Some(batch), Some("Noir"), 2.0);
        let clay = member(Some(batch), Some("Clay"), 1.0);
        let group = LibraryBatch::find(batch, &[noir.clone(), clay.clone()]).unwrap();
        let names: HashMap<Uuid, String> = [(noir.spec().preset_id.unwrap(), "Film Noir".to_owned())].into();
        let labels: Vec<String> = group.cells(&names).into_iter().map(|cell| cell.label).collect();
        assert_eq!(labels, ["Film Noir", "Style 2"]);
        assert_eq!(group.title(), "rooftop · 2 styles");
        assert_eq!(group.subtitle(now()), "Draft 480p · $3.82 · 1 min ago");
        assert_eq!(
            group.accessibility_label(now()),
            "Batch: rooftop · 2 styles, Draft 480p · $3.82 · 1 min ago"
        );

        let mut swap = group.clone();
        for item in &mut swap.items {
            item.job.spec.mode = Mode::MotionTransfer;
        }
        let labels: Vec<String> = swap.cells(&names).into_iter().map(|cell| cell.label).collect();
        assert_eq!(labels, ["Character 1", "Character 2"]);
        assert!(LibraryBatch::find(batch, &[noir]).is_none());
    }

    /// `LibraryBatchTests.eachCellPlaysItsResultUnderItsLabel`: the cell's track is its file; none without one.
    #[test]
    fn each_cell_plays_its_result_under_its_label() {
        let batch = Uuid::new_v4();
        let mut first = member(Some(batch), Some("Noir"), 2.0);
        first.job.output_file = Some(file("/home/me/Movies/Jutsu Studio/noir.mp4"));
        let mut second = member(Some(batch), Some("Clay"), 1.0);
        second.job.output_file = None;
        let group = LibraryBatch::find(batch, &[first.clone(), second]).unwrap();
        let cells = group.cells(&HashMap::new());
        assert_eq!(cells[0].label, "Style 1");
        assert_eq!(cells[0].item.output_file(), first.output_file());
        assert!(cells[1].item.output_file().is_none());
    }

    #[test]
    fn unstarred_are_the_results_nobody_starred() {
        let batch = Uuid::new_v4();
        let mut starred = member(Some(batch), None, 2.0);
        starred.is_starred = true;
        let plain = member(Some(batch), None, 1.0);
        let group = LibraryBatch::find(batch, &[starred, plain.clone()]).unwrap();
        assert!(group.is_starred());
        assert_eq!(
            group.unstarred().iter().map(|item| item.id()).collect::<Vec<_>>(),
            [plain.id()]
        );
    }
}
