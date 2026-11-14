//! Comparing images (`CompareImage`, `ImageComparePair` and `VariantCompare` in `Compare.swift`): two images side by
//! side, or a variant group judged two at a time. The shared zoom and pan run in the window, where the pane sizes are
//! known (`src/features/compare/imageCompareState.ts`).

use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

/// One image on the compare surface: a saved file, or an account image read from its link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareImage {
    pub id: Uuid,
    pub url: Url,
    pub prompt: String,
}

impl CompareImage {
    /// The line under its pane: the prompt, or the file's name without one.
    pub fn title(&self) -> String {
        if !self.prompt.is_empty() {
            return self.prompt.clone();
        }
        self.url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| !name.is_empty())
            .map_or_else(
                || self.url.to_string(),
                |name| {
                    percent_encoding::percent_decode_str(name)
                        .decode_utf8_lossy()
                        .into_owned()
                },
            )
    }
}

/// Two images compared side by side at the same zoom; `None` for the same image twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageComparePair {
    pub left: CompareImage,
    pub right: CompareImage,
}

impl ImageComparePair {
    pub fn new(left: CompareImage, right: CompareImage) -> Option<Self> {
        (left.id != right.id).then_some(Self { left, right })
    }
}

/// A variant group in the image compare: one variant stays on the left while the right side steps through the
/// others, so a group is judged two at a time without going back to the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantCompare<T> {
    variants: Vec<T>,
    left: usize,
    right: usize,
}

impl<T> VariantCompare<T> {
    /// `None` with fewer than two variants.
    pub fn new(variants: Vec<T>) -> Option<Self> {
        (variants.len() > 1).then_some(Self {
            variants,
            left: 0,
            right: 1,
        })
    }

    /// The group with `left` and `right` shown; `None` when they are the same or outside it.
    pub fn at(variants: Vec<T>, left: usize, right: usize) -> Option<Self> {
        let mut compare = Self::new(variants)?;
        if left == right || left >= compare.variants.len() || right >= compare.variants.len() {
            return None;
        }
        compare.left = left;
        compare.right = right;
        Some(compare)
    }

    pub fn pair(&self) -> (&T, &T) {
        (&self.variants[self.left], &self.variants[self.right])
    }

    pub fn left(&self) -> usize {
        self.left
    }

    pub fn right(&self) -> usize {
        self.right
    }

    /// Moves the right side `offset` variants on, wrapping around and skipping the left side's.
    pub fn step(&mut self, offset: isize) {
        let count = self.variants.len();
        for _ in 0..offset.unsigned_abs() {
            loop {
                self.right = if offset > 0 {
                    (self.right + 1) % count
                } else {
                    (self.right + count - 1) % count
                };
                if self.right != self.left {
                    break;
                }
            }
        }
    }

    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
    }

    /// "Variant 2 of 4", the right side's place in the group.
    pub fn position(&self) -> String {
        format!("Variant {} of {}", self.right + 1, self.variants.len())
    }
}

/// One image of the compare page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CompareImageView {
    pub id: String,
    /// The prompt, or the file's name; also the pane's accessibility label.
    pub title: String,
    /// Where `<img>` loads it: the media server for a file here, the account's link otherwise.
    pub url: Option<String>,
}

/// Which images the two panes show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageCompareSides {
    pub left: i32,
    pub right: i32,
    /// A variant group's pager, "Variant 2 of 4"; `None` for two images, which only swap.
    pub position: Option<String>,
}

/// What the compare page's controls ask of the sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SidesChange {
    /// Previous Variant (↑).
    Previous,
    /// Next Variant (↓).
    Next,
    /// Swap Sides (← →).
    Swap,
}

impl ImageCompareSides {
    /// Two images: the first on the left.
    pub fn pair() -> Self {
        Self {
            left: 0,
            right: 1,
            position: None,
        }
    }

    /// The first two of a group of `count` variants.
    pub fn variants(count: usize) -> Option<Self> {
        VariantCompare::new(vec![(); count]).map(|compare| Self::of(&compare))
    }

    fn of<T>(compare: &VariantCompare<T>) -> Self {
        Self {
            left: index(compare.left()),
            right: index(compare.right()),
            position: Some(compare.position()),
        }
    }

    /// The sides after `change` among `count` images; unchanged when they don't fit `count`.
    #[must_use]
    pub fn changed(self, count: usize, change: SidesChange) -> Self {
        let (Ok(left), Ok(right)) = (usize::try_from(self.left), usize::try_from(self.right)) else {
            return self;
        };
        if self.position.is_none() {
            return match change {
                SidesChange::Swap => Self {
                    left: self.right,
                    right: self.left,
                    position: None,
                },
                SidesChange::Previous | SidesChange::Next => self,
            };
        }
        let Some(mut compare) = VariantCompare::at(vec![(); count], left, right) else {
            return self;
        };
        match change {
            SidesChange::Previous => compare.step(-1),
            SidesChange::Next => compare.step(1),
            SidesChange::Swap => compare.swap(),
        }
        Self::of(&compare)
    }
}

fn index(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The page of two images or of a variant group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageCompareView {
    pub images: Vec<CompareImageView>,
    pub sides: ImageCompareSides,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(id: Uuid, name: &str) -> CompareImage {
        CompareImage {
            id,
            url: Url::from_file_path(std::env::temp_dir().join(format!("{name}.png"))).unwrap(),
            prompt: name.to_owned(),
        }
    }

    /// `ImageCompareTests.needsTwoDifferentImages`.
    #[test]
    fn needs_two_different_images() {
        let first = image(Uuid::new_v4(), "knight");
        assert!(ImageComparePair::new(first.clone(), first.clone()).is_none());
        assert!(ImageComparePair::new(first, image(Uuid::new_v4(), "dance")).is_some());
    }

    /// `VariantCompareTests.theRightSideStepsThroughTheOthers`.
    #[test]
    fn the_right_side_steps_through_the_others() {
        let images: Vec<CompareImage> = (0..3)
            .map(|index| image(Uuid::new_v4(), &format!("variant {index}")))
            .collect();
        let ids = |compare: &VariantCompare<CompareImage>| {
            let (left, right) = compare.pair();
            (left.id, right.id)
        };
        let mut compare = VariantCompare::new(images.clone()).unwrap();
        assert_eq!(ids(&compare), (images[0].id, images[1].id));
        compare.step(1);
        assert_eq!(ids(&compare), (images[0].id, images[2].id));
        compare.step(1);
        assert_eq!(ids(&compare), (images[0].id, images[1].id));
        compare.step(-1);
        assert_eq!(ids(&compare), (images[0].id, images[2].id));
        assert_eq!(compare.position(), "Variant 3 of 3");
        compare.swap();
        assert_eq!(ids(&compare), (images[2].id, images[0].id));
        assert!(VariantCompare::new(images[..1].to_vec()).is_none());
    }

    /// The pane's title: the prompt, else the file's name.
    #[test]
    fn the_title_is_the_prompt_or_the_file_name() {
        let mut knight = image(Uuid::new_v4(), "knight");
        assert_eq!(knight.title(), "knight");
        knight.prompt = String::new();
        assert_eq!(knight.title(), "knight.png");
        knight.url = Url::parse("https://cdn.example.com/out/a%20knight.webp").unwrap();
        assert_eq!(knight.title(), "a knight.webp");
    }

    /// The page's sides follow the same steps over IPC: two images only swap, a group pages the right side.
    #[test]
    fn the_sides_page_a_group_and_swap_a_pair() {
        let pair = ImageCompareSides::pair();
        assert_eq!(pair.clone().changed(2, SidesChange::Next), pair);
        let swapped = pair.changed(2, SidesChange::Swap);
        assert_eq!((swapped.left, swapped.right, swapped.position), (1, 0, None));

        let group = ImageCompareSides::variants(3).unwrap();
        assert_eq!(group.position.as_deref(), Some("Variant 2 of 3"));
        let next = group.changed(3, SidesChange::Next);
        assert_eq!((next.left, next.right), (0, 2));
        assert_eq!(next.position.as_deref(), Some("Variant 3 of 3"));
        let wrapped = next.clone().changed(3, SidesChange::Next);
        assert_eq!((wrapped.left, wrapped.right), (0, 1));
        let swapped = next.changed(3, SidesChange::Swap);
        assert_eq!((swapped.left, swapped.right), (2, 0));
        assert_eq!(swapped.position.as_deref(), Some("Variant 1 of 3"));
        let back = swapped.clone().changed(3, SidesChange::Previous);
        assert_eq!((back.left, back.right), (2, 1));
        assert!(ImageCompareSides::variants(1).is_none());
        // Sides that don't fit the group stay as they were.
        assert_eq!(swapped.clone().changed(2, SidesChange::Next), swapped);
    }
}
