//! The Motion Library as the Create screen browses it (`MotionLibraryModel.swift`, `MotionLibraryBrowser.swift`):
//! each shelf is a `PagedFeed` that loads on first view and pages on demand.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use jutsu_core::{MotionLibrary, MotionPreset, MotionShelf};
use serde::Serialize;

use super::paged_feed::{Fetch, PagedFeed};
use crate::connection::UserMessage;

/// A shelf's tab title.
pub fn shelf_title(shelf: MotionShelf) -> &'static str {
    match shelf {
        MotionShelf::Featured => "Higgsfield",
        MotionShelf::Trending => "Trending",
        MotionShelf::New => "New",
        MotionShelf::Uploads => "Your Uploads",
    }
}

/// The Motion Library's shelves, each with its own pages.
pub struct MotionLibraryModel {
    shelf: Mutex<MotionShelf>,
    feeds: HashMap<MotionShelf, PagedFeed<MotionPreset>>,
}

impl std::fmt::Debug for MotionLibraryModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MotionLibraryModel")
            .finish_non_exhaustive()
    }
}

impl MotionLibraryModel {
    pub fn new(library: &Arc<dyn MotionLibrary>) -> Self {
        let feeds = MotionShelf::ALL
            .iter()
            .map(|shelf| {
                let (library, shelf) = (library.clone(), *shelf);
                let fetch: Fetch<MotionPreset> = Arc::new(move |cursor| {
                    let library = library.clone();
                    async move {
                        let page = library.motion_presets(shelf, cursor).await?;
                        Ok((page.items, page.next_cursor))
                    }
                    .boxed()
                });
                (shelf, PagedFeed::new("Can't load the library", fetch))
            })
            .collect();
        Self {
            shelf: Mutex::new(MotionShelf::Featured),
            feeds,
        }
    }

    pub fn shelf(&self) -> MotionShelf {
        *self.shelf.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_shelf(&self, shown: MotionShelf) {
        *self.shelf.lock().unwrap_or_else(PoisonError::into_inner) = shown;
    }

    fn feed(&self) -> Option<&PagedFeed<MotionPreset>> {
        self.feeds.get(&self.shelf())
    }

    pub fn current(&self) -> Vec<MotionPreset> {
        self.feed().map(|feed| feed.view().items).unwrap_or_default()
    }

    pub fn is_loading(&self) -> bool {
        self.feed().is_some_and(|feed| feed.view().is_loading)
    }

    pub fn failure(&self) -> Option<UserMessage> {
        self.feed().and_then(|feed| feed.view().failure)
    }

    pub fn has_more(&self) -> bool {
        self.feed().is_some_and(|feed| feed.view().has_more)
    }

    /// Loads the shown shelf's first page, unless it has one.
    pub async fn load_if_needed(&self) {
        if let Some(feed) = self.feed().cloned() {
            feed.load_if_needed().await;
        }
    }

    /// The shown shelf's next page. Ignored after its last page; joins the load already running for it.
    pub async fn load_more(&self) {
        if let Some(feed) = self.feed().cloned() {
            feed.load_more().await;
        }
    }

    /// What the browser shows now.
    pub fn view(&self, downloading: Option<&str>) -> MotionLibraryView {
        let shown = self.shelf();
        let view = self.feed().map(PagedFeed::view);
        let items: Vec<MotionClipView> = view
            .as_ref()
            .map(|view| view.items.iter().map(clip_view).collect())
            .unwrap_or_default();
        let failure = view.as_ref().and_then(|view| view.failure.clone());
        let has_more = view.as_ref().is_some_and(|view| view.has_more);
        MotionLibraryView {
            shelves: MotionShelf::ALL
                .iter()
                .map(|shelf| ShelfChoice {
                    value: shelf.raw_value().to_owned(),
                    title: shelf_title(*shelf).to_owned(),
                })
                .collect(),
            shelf: shown.raw_value().to_owned(),
            // The skeleton stands for the first page.
            shows_skeleton: items.is_empty() && failure.is_none() && has_more,
            is_loading: view.as_ref().is_some_and(|view| view.is_loading),
            items,
            failure,
            has_more,
            downloading_id: downloading.map(str::to_owned),
        }
    }
}

fn clip_view(preset: &MotionPreset) -> MotionClipView {
    MotionClipView {
        id: preset.id.clone(),
        name: preset.name.clone(),
        thumbnail_url: preset.thumbnail_url.as_ref().map(ToString::to_string),
        preview_url: preset.preview_url.as_ref().map(ToString::to_string),
        example: preset
            .example_mode
            .map(|mode| super::form::mode_short_title(mode).to_owned()),
    }
}

/// A shelf's tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShelfChoice {
    pub value: String,
    pub title: String,
}

/// One clip of the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionClipView {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    /// The example, played on hover.
    pub preview_url: Option<String>,
    /// What the example did: "Motion", "Swap"…
    pub example: Option<String>,
}

/// The browser on the empty stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionLibraryView {
    pub shelves: Vec<ShelfChoice>,
    pub shelf: String,
    pub items: Vec<MotionClipView>,
    /// Ten placeholder tiles while the first page loads.
    pub shows_skeleton: bool,
    pub is_loading: bool,
    pub failure: Option<UserMessage>,
    pub has_more: bool,
    /// The clip being downloaded; the others wait for it.
    pub downloading_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use jutsu_core::{HistoryCursor, MotionPresetPage, PortError, ProviderError};
    use url::Url;

    struct FakeLibrary {
        failing: Vec<MotionShelf>,
    }

    #[async_trait]
    impl MotionLibrary for FakeLibrary {
        async fn motion_presets(
            &self,
            asked: MotionShelf,
            _after: Option<HistoryCursor>,
        ) -> Result<MotionPresetPage, PortError> {
            if self.failing.contains(&asked) {
                return Err(PortError::Provider(ProviderError::Network));
            }
            let items = (0..2)
                .map(|index| MotionPreset {
                    id: format!("{}-0-{index}", asked.raw_value()),
                    name: "Clip".to_owned(),
                    video_url: Url::parse("https://example.com/a.mp4").unwrap(),
                    thumbnail_url: None,
                    preview_url: None,
                    example_mode: None,
                })
                .collect();
            Ok(MotionPresetPage {
                items,
                next_cursor: None,
            })
        }
    }

    fn model(failing: Vec<MotionShelf>) -> MotionLibraryModel {
        let library: Arc<dyn MotionLibrary> = Arc::new(FakeLibrary { failing });
        MotionLibraryModel::new(&library)
    }

    #[tokio::test]
    async fn each_shelf_keeps_its_own_pages() {
        let model = model(Vec::new());
        model.load_if_needed().await;
        model.set_shelf(MotionShelf::Trending);
        assert_eq!(model.current().len(), 0);
        model.load_if_needed().await;
        assert_eq!(
            model.current().first().map(|clip| clip.id.as_str()),
            Some("trending-0-0")
        );
        model.set_shelf(MotionShelf::Featured);
        assert_eq!(model.current().len(), 2);
    }

    #[tokio::test]
    async fn a_failure_stays_on_its_shelf() {
        let model = model(vec![MotionShelf::New]);
        model.set_shelf(MotionShelf::New);
        model.load_if_needed().await;
        assert!(model.failure().is_some());
        assert!(model.has_more());
        model.set_shelf(MotionShelf::Featured);
        model.load_if_needed().await;
        assert!(model.failure().is_none());
        assert!(!model.is_loading());
    }
}
