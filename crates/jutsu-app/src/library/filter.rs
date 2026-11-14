//! The Library's filter bar (`LibraryFilter.swift`, `LibraryFilterOptions.swift`): search, the video and image
//! filters, starred, show rejected and the order, and the menus' values.

use std::collections::BTreeSet;

use jutsu_core::{Mode, Resolution, VideoModel};
use serde::{Deserialize, Serialize};

use super::culling::CatalogIndex;
use super::item::LibraryItem;
use super::media::{KindCounts, LibraryKind, MediaRow};
use crate::connection::mode_title;

/// Draft or final: videos only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Finish {
    #[default]
    All,
    Drafts,
    Finals,
}

/// The order of the grid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySort {
    #[default]
    Newest,
    Oldest,
    /// Dollars before credits, the most expensive first; a video made on Higgsfield and every image last.
    HighestPrice,
}

/// One control of the filter bar, in bar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum FilterField {
    Mode,
    Model,
    Resolution,
    Finish,
    ImageModel,
    ImageAspectRatio,
    Starred,
    Rejected,
    Sort,
}

impl FilterField {
    /// The fields the bar can show for `kind`: video filters hide with Images, image filters with Videos.
    pub fn visible(kind: LibraryKind) -> &'static [Self] {
        use FilterField::{
            Finish, ImageAspectRatio, ImageModel, Mode, Model, Rejected, Resolution, Sort, Starred,
        };
        match kind {
            LibraryKind::All => &[
                Mode,
                Model,
                Resolution,
                Finish,
                ImageModel,
                ImageAspectRatio,
                Starred,
                Rejected,
                Sort,
            ],
            LibraryKind::Video => &[Mode, Model, Resolution, Finish, Starred, Rejected, Sort],
            LibraryKind::Image => &[ImageModel, ImageAspectRatio, Starred, Rejected, Sort],
        }
    }
}

/// Search, filters and order. Mode, model, resolution and finish apply to videos only; image model and aspect ratio
/// to images only; search, starred and sort to both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryFilter {
    pub query: String,
    pub mode: Option<Mode>,
    pub model: Option<VideoModel>,
    pub resolution: Option<Resolution>,
    pub finish: Finish,
    pub image_model: Option<String>,
    pub image_aspect_ratio: Option<String>,
    /// Starred here, or liked on Higgsfield.
    pub is_starred_only: bool,
    /// Rejected results stay out of every tab and every search until this is on.
    pub shows_rejected: bool,
    pub sort: LibrarySort,
}

impl LibraryFilter {
    pub fn matches_video(&self, item: &LibraryItem) -> bool {
        let spec = item.spec();
        if self.is_starred_only && !item.is_starred {
            return false;
        }
        if self.mode.is_some_and(|mode| spec.mode != mode)
            || self.model.as_ref().is_some_and(|model| &spec.model != model)
            || self
                .resolution
                .is_some_and(|resolution| spec.resolution != resolution)
        {
            return false;
        }
        if self.finish != Finish::All && spec.is_draft != (self.finish == Finish::Drafts) {
            return false;
        }
        let needle = self.query.trim().to_lowercase();
        needle.is_empty()
            || format!("{} {}", spec.prompt, item.title())
                .to_lowercase()
                .contains(&needle)
    }

    /// An image matches when the search finds every word in its prompt or model; video filters never touch it.
    pub fn matches_image(&self, row: &MediaRow) -> bool {
        if self.is_starred_only && !row.is_starred() {
            return false;
        }
        if let Some(model) = &self.image_model
            && row.image_model() != Some(model.as_str())
        {
            return false;
        }
        if let Some(aspect) = &self.image_aspect_ratio
            && row.aspect_ratio().as_ref() != Some(aspect)
        {
            return false;
        }
        let query = self.query.to_lowercase();
        let text = row.search_text().to_lowercase();
        query.split_whitespace().all(|word| text.contains(word))
    }

    pub fn matches(&self, row: &MediaRow) -> bool {
        match row {
            MediaRow::Video(item) => self.matches_video(item),
            MediaRow::LocalImage(_) | MediaRow::RemoteImage(_) => self.matches_image(row),
        }
    }

    /// The matching videos in `sort` order; `items` come newest first.
    pub fn apply_videos(&self, items: &[LibraryItem]) -> Vec<LibraryItem> {
        let matching: Vec<LibraryItem> = items
            .iter()
            .filter(|item| self.matches_video(item))
            .cloned()
            .collect();
        match self.sort {
            LibrarySort::Newest => matching,
            LibrarySort::Oldest => matching.into_iter().rev().collect(),
            LibrarySort::HighestPrice => by_price(matching, LibraryItem::price_key),
        }
    }

    /// The matching rows of `kind` in `sort` order: with All, videos and images interleave by date.
    pub fn apply(&self, rows: &[MediaRow], kind: LibraryKind) -> Vec<MediaRow> {
        let mut matching: Vec<MediaRow> = rows
            .iter()
            .filter(|row| self.matches(row) && of_kind(row, kind))
            .cloned()
            .collect();
        match self.sort {
            LibrarySort::Newest => {
                matching.sort_by_key(|row| std::cmp::Reverse(row.created_at()));
                matching
            }
            LibrarySort::Oldest => {
                matching.sort_by_key(MediaRow::created_at);
                matching
            }
            LibrarySort::HighestPrice => by_price(matching, |row| {
                row.video()
                    .map_or((2, rust_decimal::Decimal::ZERO), LibraryItem::price_key)
            }),
        }
    }

    /// `rows` without the rejected ones unless they are shown; runs before the search, the filters and the counts.
    pub fn culled(&self, rows: Vec<MediaRow>, index: &CatalogIndex) -> Vec<MediaRow> {
        if self.shows_rejected {
            return rows;
        }
        rows.into_iter()
            .filter(|row| !index.is_rejected(row.id()))
            .collect()
    }

    /// The kind tabs' counts: every filter but the kind applies.
    pub fn counts(&self, rows: &[MediaRow]) -> KindCounts {
        let (mut videos, mut images) = (0, 0);
        for row in rows.iter().filter(|row| self.matches(row)) {
            if row.is_video() {
                videos += 1;
            } else {
                images += 1;
            }
        }
        KindCounts::new(videos, images)
    }

    /// Whether search or a filter hides anything; the order and showing rejected results don't count.
    pub fn is_active(&self) -> bool {
        let mut unsorted = self.clone();
        unsorted.sort = LibrarySort::Newest;
        unsorted.shows_rejected = false;
        unsorted != Self::default()
    }

    /// The same order with every filter and the search cleared.
    pub fn cleared(&self) -> Self {
        Self {
            sort: self.sort,
            ..Self::default()
        }
    }
}

fn of_kind(row: &MediaRow, kind: LibraryKind) -> bool {
    match kind {
        LibraryKind::All => true,
        LibraryKind::Video => row.is_video(),
        LibraryKind::Image => !row.is_video(),
    }
}

/// Highest first by `(rank, amount)`: lower rank first, then the larger amount; ties keep their place.
fn by_price<T>(rows: Vec<T>, key: impl Fn(&T) -> (u8, rust_decimal::Decimal)) -> Vec<T> {
    let mut keyed: Vec<((u8, rust_decimal::Decimal), T)> =
        rows.into_iter().map(|row| (key(&row), row)).collect();
    keyed.sort_by(|(left, _), (right, _)| left.0.cmp(&right.0).then(right.1.cmp(&left.1)));
    keyed.into_iter().map(|(_, row)| row).collect()
}

/// One value of a filter menu.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FilterChoice {
    /// What the filter holds: `Mode`, `VideoModel` or `Resolution`'s raw value.
    pub value: String,
    pub title: String,
}

/// What the filter menus offer: only the values the origin's rows have. A menu with fewer than two values is left
/// out unless it holds one that can still be cleared.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FilterOptions {
    pub modes: Vec<FilterChoice>,
    pub models: Vec<FilterChoice>,
    pub resolutions: Vec<FilterChoice>,
    pub has_drafts_and_finals: bool,
    pub image_models: Vec<String>,
    pub image_aspects: Vec<String>,
}

impl FilterOptions {
    /// `rows`: the origin's rows before any filter, so a menu keeps its values while one is picked.
    pub fn new(rows: &[MediaRow]) -> Self {
        let mut modes = BTreeSet::new();
        let mut video_models = Vec::<VideoModel>::new();
        let mut resolutions = BTreeSet::new();
        let mut finishes = BTreeSet::new();
        let mut image_models = BTreeSet::new();
        let mut image_aspects = BTreeSet::new();
        for row in rows {
            if let Some(item) = row.video() {
                let spec = item.spec();
                modes.insert(spec.mode);
                if !video_models.contains(&spec.model) {
                    video_models.push(spec.model.clone());
                }
                resolutions.insert(spec.resolution);
                finishes.insert(spec.is_draft);
            } else {
                image_models.extend(row.image_model().map(str::to_owned));
                image_aspects.extend(row.aspect_ratio());
            }
        }
        let catalog = VideoModel::all();
        let mut ordered: Vec<VideoModel> = catalog
            .into_iter()
            .filter(|model| video_models.contains(model))
            .collect();
        ordered.extend(
            video_models
                .into_iter()
                .filter(|model| !ordered.contains(model))
                .collect::<Vec<_>>(),
        );
        Self {
            modes: Mode::ALL
                .iter()
                .filter(|mode| modes.contains(*mode))
                .map(|mode| FilterChoice {
                    value: mode.raw_value().to_owned(),
                    title: mode_title(*mode).to_owned(),
                })
                .collect(),
            models: ordered
                .iter()
                .map(|model| FilterChoice {
                    value: model.raw_value().to_owned(),
                    title: model.title(),
                })
                .collect(),
            resolutions: Resolution::ALL
                .iter()
                .filter(|resolution| resolutions.contains(*resolution))
                .map(|resolution| FilterChoice {
                    value: resolution.raw_value().to_owned(),
                    title: resolution.raw_value().to_owned(),
                })
                .collect(),
            has_drafts_and_finals: finishes.len() == 2,
            image_models: image_models.into_iter().collect(),
            image_aspects: image_aspects.into_iter().collect(),
        }
    }

    /// Whether `field`'s menu shows: it has a choice to make, or it holds one to undo.
    pub fn shows(&self, field: FilterField, filter: &LibraryFilter) -> bool {
        match field {
            FilterField::Mode => self.modes.len() > 1 || filter.mode.is_some(),
            FilterField::Model => self.models.len() > 1 || filter.model.is_some(),
            FilterField::Resolution => self.resolutions.len() > 1 || filter.resolution.is_some(),
            FilterField::Finish => self.has_drafts_and_finals || filter.finish != Finish::All,
            FilterField::ImageModel => self.image_models.len() > 1 || filter.image_model.is_some(),
            FilterField::ImageAspectRatio => {
                self.image_aspects.len() > 1 || filter.image_aspect_ratio.is_some()
            }
            FilterField::Starred | FilterField::Rejected | FilterField::Sort => true,
        }
    }

    /// The fields the bar shows for `kind` with `filter`.
    pub fn fields(&self, kind: LibraryKind, filter: &LibraryFilter) -> Vec<FilterField> {
        FilterField::visible(kind)
            .iter()
            .copied()
            .filter(|field| self.shows(*field, filter))
            .collect()
    }
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
