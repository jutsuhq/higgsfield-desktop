//! References › From Higgsfield (`UploadsPicker.swift`, `ElementsPicker.swift`): the account's uploaded images and
//! its elements, as the sheet shows them.

use jutsu_core::{AccountElement, CharacterSheet, ElementCategory, UploadedFile};
use serde::{Deserialize, Serialize};

use super::elements::{AccountElementsModel, category_title};
use super::paged_feed::FeedView;
use crate::connection::UserMessage;

/// A tab of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum HiggsfieldSource {
    Uploads,
    Elements,
    Characters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HiggsfieldSourceChoice {
    pub value: HiggsfieldSource,
    pub title: String,
}

/// One uploaded image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadTile {
    pub id: String,
    pub url: String,
}

/// The Uploads tab.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadsPanel {
    /// "Your uploads on Higgsfield".
    pub title: String,
    pub detail: String,
    pub tiles: Vec<UploadTile>,
    pub failure: Option<UserMessage>,
    pub has_more: bool,
    /// The account has uploaded nothing: "No uploaded images yet".
    pub is_empty: bool,
}

/// One element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ElementTile {
    pub id: String,
    /// "Untitled" for an element without a name.
    pub name: String,
    pub thumbnail_url: Option<String>,
    /// Its category: "Character"…
    pub tag: Option<String>,
}

/// A choice of the Category menu; `None` is All categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ElementCategoryChoice {
    pub value: Option<String>,
    pub title: String,
}

/// The Elements tab.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ElementsPanel {
    /// Shown instead of the grid while the elements can't be browsed.
    pub unavailable_reason: Option<String>,
    pub detail: String,
    pub category: Option<String>,
    pub categories: Vec<ElementCategoryChoice>,
    pub tiles: Vec<ElementTile>,
    pub failure: Option<UserMessage>,
    pub has_more: bool,
    /// "No elements yet".
    pub is_empty: bool,
}

/// The sheet.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HiggsfieldReferencesView {
    /// The tabs; one tab shows no switch.
    pub sources: Vec<HiggsfieldSourceChoice>,
    /// How many more references the form takes.
    pub limit: u32,
    pub uploads: UploadsPanel,
    /// `None` without a provider that keeps elements.
    pub elements: Option<ElementsPanel>,
    /// `None` without an account that makes character sheets.
    pub characters: Option<CharactersPanel>,
    /// What is being downloaded: `"uploads"` or an element's ID.
    pub adding: Option<String>,
}

/// One character sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSheetTile {
    pub id: String,
    /// The copy saved in Library › Images; `None` shows `url`.
    pub path: Option<String>,
    pub url: String,
}

/// The Characters tab (`CharacterSheetsPicker`): the sheets made here, then the account's others, with Create
/// Character… on top.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharactersPanel {
    pub title: String,
    pub detail: String,
    /// Sheets still being made, as placeholders.
    pub pending: u32,
    pub tiles: Vec<CharacterSheetTile>,
    /// The account's own list is read: its next page and failure show; otherwise `note` says why only the
    /// sheets made here are listed.
    pub lists_account: bool,
    pub note: Option<String>,
    pub failure: Option<UserMessage>,
    pub has_more: bool,
    /// "No characters yet".
    pub is_empty: bool,
    /// Why a sheet can't be picked: the references are full.
    pub full_hint: Option<String>,
}

/// What picking an element did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ElementOpening {
    /// Its images were added; the sheet closes.
    Added,
    /// It has more images than fit: the user picks, starting from the first `picked`.
    Choose {
        name: String,
        detail: String,
        images: Vec<String>,
        picked: Vec<u32>,
        limit: u32,
    },
    /// Nothing was added, and why. Also a download closed by the sheet: `message` is then empty.
    Failed { message: String },
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Whether a feed has nothing to show and nothing to wait for.
fn is_empty<T>(view: &FeedView<T>) -> bool {
    view.items.is_empty() && view.failure.is_none() && !view.has_more
}

pub(crate) fn uploads_panel(view: &FeedView<UploadedFile>, limit: usize) -> UploadsPanel {
    UploadsPanel {
        title: "Your uploads on Higgsfield".to_owned(),
        detail: format!("Images you uploaded before, newest first. Pick up to {limit} to use as references."),
        tiles: view
            .items
            .iter()
            .map(|upload| UploadTile {
                id: upload.id.clone(),
                url: upload.url.to_string(),
            })
            .collect(),
        failure: view.failure.clone(),
        has_more: view.has_more,
        is_empty: is_empty(view),
    }
}

pub(crate) fn element_tile(element: &AccountElement) -> ElementTile {
    ElementTile {
        id: element.id.clone(),
        name: if element.name.is_empty() {
            "Untitled".to_owned()
        } else {
            element.name.clone()
        },
        thumbnail_url: element.thumbnail_url.as_ref().map(ToString::to_string),
        tag: element
            .category
            .map(|category| category_title(category).to_owned()),
    }
}

pub(crate) fn elements_panel(model: &AccountElementsModel, limit: usize) -> ElementsPanel {
    let view = model.feed_view();
    let mut categories = vec![ElementCategoryChoice {
        value: None,
        title: "All categories".to_owned(),
    }];
    categories.extend(ElementCategory::ALL.iter().map(|category| ElementCategoryChoice {
        value: Some(category.raw_value().to_owned()),
        title: category_title(*category).to_owned(),
    }));
    let tiles: Vec<ElementTile> = model.current().iter().map(element_tile).collect();
    let failure = view.as_ref().and_then(|view| view.failure.clone());
    let has_more = view.as_ref().is_some_and(|view| view.has_more);
    ElementsPanel {
        unavailable_reason: model.unavailable_reason().map(str::to_owned),
        detail: format!("Pick one to add its images as references, up to {limit}."),
        category: model.category().map(|category| category.raw_value().to_owned()),
        categories,
        is_empty: tiles.is_empty() && failure.is_none() && !has_more,
        tiles,
        failure,
        has_more,
    }
}

/// The question for an element with more images than fit.
pub(crate) fn choosing(element: &AccountElement, limit: usize) -> ElementOpening {
    ElementOpening::Choose {
        name: element_tile(element).name,
        detail: format!("It has more images than fit. Pick up to {limit}."),
        images: element.images.iter().map(|image| image.url.to_string()).collect(),
        picked: (0..element.images.len().min(limit)).map(count).collect(),
        limit: count(limit),
    }
}

pub(crate) fn limit_count(limit: usize) -> u32 {
    count(limit)
}

/// The sheets made here, then the account's others when its list is read.
pub(crate) fn character_sheets(
    made: Vec<CharacterSheet>,
    account: Option<&FeedView<CharacterSheet>>,
) -> Vec<CharacterSheet> {
    let mut sheets = made;
    if let Some(account) = account {
        let others: Vec<CharacterSheet> = account
            .items
            .iter()
            .filter(|sheet| !sheets.iter().any(|made| made.id == sheet.id))
            .cloned()
            .collect();
        sheets.extend(others);
    }
    sheets
}

/// The Characters tab. `account` is `None` when the account's own list isn't read.
pub(crate) fn characters_panel(
    made: Vec<CharacterSheet>,
    account: Option<&FeedView<CharacterSheet>>,
    pending: usize,
    limit: usize,
) -> CharactersPanel {
    let sheets = character_sheets(made, account);
    let has_more = account.is_some_and(|view| view.has_more);
    CharactersPanel {
        title: "Characters".to_owned(),
        detail: "Character sheets of your account. Pick one to use it as a reference image.".to_owned(),
        pending: count(pending),
        is_empty: sheets.is_empty() && pending == 0 && !has_more,
        tiles: sheets
            .iter()
            .map(|sheet| CharacterSheetTile {
                id: sheet.id.clone(),
                path: sheet
                    .file_url
                    .as_ref()
                    .and_then(|file| file.to_file_path().ok())
                    .filter(|path| path.exists())
                    .map(|path| path.to_string_lossy().into_owned()),
                url: sheet
                    .thumbnail_url
                    .as_ref()
                    .unwrap_or(&sheet.image_url)
                    .to_string(),
            })
            .collect(),
        lists_account: account.is_some(),
        note: account.is_none().then(|| {
            "Only the characters made in Jutsu Studio are listed. Sheets made on higgsfield.ai aren't shown yet."
                .to_owned()
        }),
        failure: account.and_then(|view| view.failure.clone()),
        has_more,
        full_hint: (limit == 0).then(|| "References are full.".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jutsu_core::{ElementImage, UploadKind};
    use url::Url;

    fn url(path: &str) -> Url {
        Url::parse(&format!("https://example.com/{path}")).unwrap()
    }

    #[test]
    fn an_account_without_uploads_says_so() {
        let empty = FeedView::<UploadedFile> {
            items: Vec::new(),
            is_loading: false,
            failure: None,
            has_more: false,
            has_loaded: true,
        };
        let panel = uploads_panel(&empty, 3);
        assert!(panel.is_empty);
        assert_eq!(
            panel.detail,
            "Images you uploaded before, newest first. Pick up to 3 to use as references."
        );
        let loading = FeedView {
            has_more: true,
            items: vec![UploadedFile {
                id: "m1".to_owned(),
                kind: UploadKind::Image,
                url: url("a.png"),
                thumbnail_url: None,
                pixel_size: None,
                duration: None,
                created_at: time::OffsetDateTime::UNIX_EPOCH,
            }],
            ..empty
        };
        let panel = uploads_panel(&loading, 3);
        assert!(!panel.is_empty);
        assert_eq!(panel.tiles[0].url, "https://example.com/a.png");
    }

    #[test]
    fn an_element_with_more_images_than_fit_picks_the_first_ones() {
        let element = AccountElement {
            id: "e1".to_owned(),
            name: String::new(),
            category: Some(ElementCategory::Prop),
            thumbnail_url: None,
            images: (0..4)
                .map(|index| ElementImage {
                    url: url(&format!("{index}.png")),
                    media: None,
                })
                .collect(),
            status: None,
        };
        let tile = element_tile(&element);
        assert_eq!(tile.name, "Untitled");
        assert_eq!(tile.tag.as_deref(), Some("Prop"));
        let ElementOpening::Choose {
            picked,
            limit,
            detail,
            ..
        } = choosing(&element, 2)
        else {
            panic!("expected a choice");
        };
        assert_eq!(picked, [0, 1]);
        assert_eq!(limit, 2);
        assert_eq!(detail, "It has more images than fit. Pick up to 2.");
    }

    fn sheet(id: &str) -> CharacterSheet {
        CharacterSheet {
            id: id.to_owned(),
            image_url: url(&format!("{id}.png")),
            thumbnail_url: None,
            created_at: None,
            file_url: None,
        }
    }

    /// `CharacterSheetsPicker`: the sheets made here first, the account's others after them, each once.
    #[test]
    fn the_characters_tab_lists_the_sheets_made_here_first() {
        let account = FeedView {
            items: vec![sheet("job-2"), sheet("job-9")],
            is_loading: false,
            failure: None,
            has_more: true,
            has_loaded: true,
        };
        let panel = characters_panel(vec![sheet("job-1"), sheet("job-2")], Some(&account), 1, 3);
        assert_eq!(
            panel
                .tiles
                .iter()
                .map(|tile| tile.id.as_str())
                .collect::<Vec<_>>(),
            ["job-1", "job-2", "job-9"]
        );
        assert_eq!(panel.pending, 1);
        assert!(panel.lists_account);
        assert_eq!(panel.note, None);
        assert!(!panel.is_empty);
        assert_eq!(panel.full_hint, None);
        assert_eq!(panel.tiles[0].url, "https://example.com/job-1.png");

        let only_made = characters_panel(Vec::new(), None, 0, 0);
        assert!(only_made.is_empty);
        assert!(!only_made.lists_account);
        assert_eq!(
            only_made.note.as_deref(),
            Some(
                "Only the characters made in Jutsu Studio are listed. Sheets made on higgsfield.ai aren't shown yet."
            )
        );
        assert_eq!(only_made.full_hint.as_deref(), Some("References are full."));
    }
}
