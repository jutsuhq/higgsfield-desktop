//! The account's reference elements as References › From Higgsfield browses them (`AccountElementsModel.swift`,
//! `CharacterSaving.swift`): one `PagedFeed`, kept for the connection like the Motion Library's, loaded on first view
//! and paged on demand; and a batch character set published as an element ("Save to Higgsfield…").

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use jutsu_core::{
    AccountElement, AccountElements, ElementCategory, ElementImage, LocalImage, PortError, ProviderError,
    ProviderKind, RemoteAsset,
};
use serde::{Deserialize, Serialize};

use super::paged_feed::{FeedView, Fetch, PagedFeed};
use crate::connection::UserMessage;

/// `PostOperationText.waitingForCatalog`.
pub const WAITING_FOR_CATALOG: &str = "Waiting for the account's tool catalog to be confirmed";

/// A character set published to an account as a reference element, so it isn't published twice.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SavedElement {
    pub id: String,
    /// `GenerationProvider::owner` of the account it was published to.
    pub owner: Option<String>,
}

impl SavedElement {
    /// The most characters an element's name takes ("max 32 chars").
    pub const NAME_LIMIT: usize = 32;
    /// "Save to Higgsfield…".
    pub const SAVE_TITLE: &str = "Save to Higgsfield…";
    /// Shown on a set once it is published.
    pub const SAVED_TITLE: &str = "Saved to Higgsfield";
    /// The title of a failed save.
    pub const FAILURE_TITLE: &str = "Can't save to Higgsfield";

    /// `name` without surrounding spaces, cut to `NAME_LIMIT`.
    pub fn name(name: &str) -> String {
        name.trim()
            .chars()
            .take(Self::NAME_LIMIT)
            .collect::<String>()
            .trim()
            .to_owned()
    }

    /// The dialog's explanation: what saving does and that it is free.
    pub fn save_explanation(image_count: usize) -> String {
        format!(
            "Uploads {image_count} images and saves them as a character element on your Higgsfield account, for \
             References › From Higgsfield and the web app. Free, no credits are spent."
        )
    }

    /// What a failed save tells the user, in the words of the account's errors under `FAILURE_TITLE`; one that leads
    /// somewhere (sign in again) keeps its own title. `None` when the save was cancelled.
    pub fn failure(error: &PortError) -> Option<UserMessage> {
        let mut message = UserMessage::from_error(error, Some(ProviderKind::Account))?;
        if message.action.is_none() {
            Self::FAILURE_TITLE.clone_into(&mut message.title);
        }
        Some(message)
    }
}

/// What picking an element does with its images, given how many more references the form takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementPick {
    /// Every image fits: they are added in the element's order.
    Add(Vec<ElementImage>),
    /// More images than fit: the user picks up to `limit`, starting from the first ones.
    Choose(Vec<ElementImage>, usize),
    /// Nothing can be added: the element has no images, or the form is full.
    Unavailable(String),
}

impl ElementPick {
    pub fn of(element: &AccountElement, limit: usize) -> Self {
        if element.images.is_empty() {
            return Self::Unavailable("This element has no images to use.".to_owned());
        }
        if limit == 0 {
            return Self::Unavailable("References are full.".to_owned());
        }
        if element.images.len() <= limit {
            Self::Add(element.images.clone())
        } else {
            Self::Choose(element.images.clone(), limit)
        }
    }
}

/// "Character", "Environment", "Prop".
pub fn category_title(category: ElementCategory) -> &'static str {
    match category {
        ElementCategory::Character => "Character",
        ElementCategory::Environment => "Environment",
        ElementCategory::Prop => "Prop",
    }
}

/// The account's elements. Elements whose calls aren't confirmed are never fetched.
pub struct AccountElementsModel {
    source: Arc<dyn AccountElements>,
    feed: Option<PagedFeed<AccountElement>>,
    /// `None` shows every category.
    category: Mutex<Option<ElementCategory>>,
}

impl std::fmt::Debug for AccountElementsModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountElementsModel")
            .finish_non_exhaustive()
    }
}

impl AccountElementsModel {
    /// `is_offered`: whether the provider is known to take the calls (always, for the Higgsfield account).
    pub fn new(source: Arc<dyn AccountElements>, is_offered: bool) -> Self {
        let feed = is_offered.then(|| {
            let lister = source.clone();
            let fetch: Fetch<AccountElement> = Arc::new(move |cursor| {
                let lister = lister.clone();
                async move {
                    let page = lister.elements(cursor).await?;
                    Ok((page.items, page.next_cursor))
                }
                .boxed()
            });
            PagedFeed::new("Can't load your elements", fetch)
        });
        Self {
            source,
            feed,
            category: Mutex::new(None),
        }
    }

    pub fn owner(&self) -> Option<String> {
        self.source.owner()
    }

    /// Why the elements can't be browsed; `None` when they can.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        self.feed.is_none().then_some(WAITING_FOR_CATALOG)
    }

    pub fn category(&self) -> Option<ElementCategory> {
        *self.category.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_category(&self, category: Option<ElementCategory>) {
        *self.category.lock().unwrap_or_else(PoisonError::into_inner) = category;
    }

    pub fn feed_view(&self) -> Option<FeedView<AccountElement>> {
        self.feed.as_ref().map(PagedFeed::view)
    }

    /// The elements, filtered by the category.
    pub fn current(&self) -> Vec<AccountElement> {
        let items = self.feed_view().map(|view| view.items).unwrap_or_default();
        match self.category() {
            Some(category) => items
                .into_iter()
                .filter(|element| element.category == Some(category))
                .collect(),
            None => items,
        }
    }

    pub async fn load_if_needed(&self) {
        if let Some(feed) = &self.feed {
            feed.load_if_needed().await;
        }
    }

    pub async fn load_more(&self) {
        if let Some(feed) = &self.feed {
            feed.load_more().await;
        }
    }

    /// Whether a character set can be published as an element ("Save to Higgsfield…").
    pub fn can_publish(&self) -> bool {
        self.feed.is_some()
    }

    /// Uploads `images` with `upload`, in order, and publishes them as a new character element named `name`, which
    /// is then offered first in the list. Free. Sent once: a failure after the call may have reached the server can
    /// still have made the element, which the list shows on its next load.
    pub async fn publish<F, Fut>(
        &self,
        images: &[LocalImage],
        name: &str,
        upload: F,
    ) -> Result<SavedElement, PortError>
    where
        F: Fn(LocalImage) -> Fut,
        Fut: Future<Output = Result<RemoteAsset, PortError>>,
    {
        let Some(feed) = &self.feed else {
            return Err(PortError::Provider(ProviderError::Validation {
                field: None,
                message: WAITING_FOR_CATALOG.to_owned(),
            }));
        };
        let mut assets = Vec::with_capacity(images.len());
        for image in images {
            assets.push(upload(image.clone()).await?);
        }
        let element = self
            .source
            .create_element(SavedElement::name(name), ElementCategory::Character, assets)
            .await?;
        let saved = SavedElement {
            id: element.id.clone(),
            owner: self.source.owner(),
        };
        feed.insert(element);
        Ok(saved)
    }

    /// `element` with its images, asking the provider when its page named none.
    pub async fn with_images(&self, element: AccountElement) -> Result<AccountElement, PortError> {
        if !element.images.is_empty() || self.feed.is_none() {
            return Ok(element);
        }
        self.source.element_images(element).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use jutsu_core::{AccountElementPage, HistoryCursor};
    use url::Url;

    #[derive(Default)]
    struct FakeElements {
        requests: Mutex<Vec<String>>,
        fails: bool,
    }

    fn image(index: usize) -> ElementImage {
        ElementImage {
            url: Url::parse(&format!("https://cdn.example.com/{index}.jpg")).unwrap(),
            media: None,
        }
    }

    fn element(
        id: &str,
        name: &str,
        category: Option<ElementCategory>,
        images: Vec<ElementImage>,
    ) -> AccountElement {
        AccountElement {
            id: id.to_owned(),
            name: name.to_owned(),
            category,
            thumbnail_url: None,
            images,
            status: None,
        }
    }

    #[async_trait]
    impl AccountElements for FakeElements {
        fn owner(&self) -> Option<String> {
            Some("user_1".to_owned())
        }

        async fn elements(&self, _after: Option<HistoryCursor>) -> Result<AccountElementPage, PortError> {
            self.requests.lock().unwrap().push("list elements".to_owned());
            Ok(AccountElementPage {
                items: vec![
                    element("el-1", "Mara", Some(ElementCategory::Character), Vec::new()),
                    element("el-2", "Roof", Some(ElementCategory::Environment), vec![image(1)]),
                ],
                next_cursor: None,
            })
        }

        async fn element_images(&self, element: AccountElement) -> Result<AccountElement, PortError> {
            self.requests.lock().unwrap().push(format!("get {}", element.id));
            let mut full = element;
            full.images = vec![image(1), image(1)];
            Ok(full)
        }

        async fn create_element(
            &self,
            name: String,
            category: ElementCategory,
            images: Vec<RemoteAsset>,
        ) -> Result<AccountElement, PortError> {
            if self.fails {
                return Err(PortError::Provider(ProviderError::Server { status: 500 }));
            }
            let ids: Vec<String> = images.iter().filter_map(|asset| asset.id.clone()).collect();
            self.requests
                .lock()
                .unwrap()
                .push(format!("create {name} {} {ids:?}", category.raw_value()));
            Ok(element("el-new", &name, Some(category), Vec::new()))
        }
    }

    fn requests(source: &FakeElements) -> Vec<String> {
        source.requests.lock().unwrap().clone()
    }

    fn ids(model: &AccountElementsModel) -> Vec<String> {
        model.current().into_iter().map(|element| element.id).collect()
    }

    fn local(name: &str) -> LocalImage {
        LocalImage::new(crate::testing::file_url(format!("/tmp/{name}")))
    }

    fn asset(image: &LocalImage, id: String) -> RemoteAsset {
        RemoteAsset {
            url: Url::parse(&format!("https://upload.example/{}", image.file_url.path())).unwrap(),
            id: Some(id),
        }
    }

    #[tokio::test]
    async fn unconfirmed_elements_are_never_fetched() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), false);
        model.load_if_needed().await;
        model
            .with_images(element("el-1", "Mara", None, Vec::new()))
            .await
            .unwrap();
        assert_eq!(model.unavailable_reason(), Some(WAITING_FOR_CATALOG));
        assert_eq!(model.current().len(), 0);
        assert_eq!(requests(&source).len(), 0);
    }

    #[tokio::test]
    async fn the_category_filters_the_loaded_list() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), true);
        model.load_if_needed().await;
        assert_eq!(model.unavailable_reason(), None);
        assert_eq!(ids(&model), ["el-1", "el-2"]);
        model.set_category(Some(ElementCategory::Environment));
        assert_eq!(ids(&model), ["el-2"]);
        assert_eq!(requests(&source), ["list elements"]);
    }

    #[tokio::test]
    async fn images_are_asked_for_only_when_the_list_named_none() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), true);
        model.load_if_needed().await;
        let current = model.current();
        let bare = model.with_images(current[0].clone()).await.unwrap();
        let listed = model.with_images(current[1].clone()).await.unwrap();
        assert_eq!(bare.images.len(), 2);
        assert_eq!(listed.images.len(), 1);
        assert_eq!(requests(&source), ["list elements", "get el-1"]);
    }

    #[tokio::test]
    async fn publishing_uploads_each_image_then_creates_a_character_offered_first() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), true);
        model.load_if_needed().await;
        let images = [local("1.png"), local("2.png")];
        let uploaded = Mutex::new(Vec::new());
        let saved = model
            .publish(&images, "  Mara  ", |image| {
                let mut list = uploaded.lock().unwrap();
                list.push(image.file_url.clone());
                let id = format!("m-{}", list.len());
                let asset = asset(&image, id);
                async move { Ok(asset) }
            })
            .await
            .unwrap();
        assert_eq!(
            saved,
            SavedElement {
                id: "el-new".to_owned(),
                owner: Some("user_1".to_owned())
            }
        );
        let files: Vec<Url> = images.iter().map(|image| image.file_url.clone()).collect();
        assert_eq!(*uploaded.lock().unwrap(), files);
        assert_eq!(
            requests(&source),
            ["list elements", r#"create Mara character ["m-1", "m-2"]"#]
        );
        assert_eq!(ids(&model), ["el-new", "el-1", "el-2"]);
    }

    #[tokio::test]
    async fn nothing_is_uploaded_or_published_while_elements_arent_offered() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), false);
        let uploaded = Mutex::new(0);
        assert!(!model.can_publish());
        let result = model
            .publish(&[local("1.png")], "Mara", |image| {
                *uploaded.lock().unwrap() += 1;
                let asset = asset(&image, "m-1".to_owned());
                async move { Ok(asset) }
            })
            .await;
        assert!(matches!(result, Err(PortError::Provider(_))));
        assert_eq!(*uploaded.lock().unwrap(), 0);
        assert_eq!(requests(&source).len(), 0);
    }

    /// `CharacterSavingTests.thereIsNoSaveWithoutElementsThatPublishOrAnUpload`: elements the user hasn't confirmed
    /// don't publish. Without elements or a connection to upload with, `CreateService::save_character_set` saves
    /// nothing (`service_tests::an_api_key_has_no_save_to_higgsfield`).
    /// `CharacterSavingTests.aSavedSetIsRecordedForItsImages`: the element is recorded on the draft for the set's
    /// files and the account that owns it, as `CreateService::save_character_set` does.
    #[tokio::test]
    async fn a_saved_set_is_recorded_for_its_images() {
        let source = Arc::new(FakeElements::default());
        let model = AccountElementsModel::new(source.clone(), true);
        let images = [local("a.png"), local("b.png")];
        let saved = model
            .publish(&images, "  Mara  ", |image| {
                let id = image.file_url.path().trim_start_matches("/tmp/").to_owned();
                let asset = asset(&image, id);
                async move { Ok(asset) }
            })
            .await
            .unwrap();
        assert_eq!(requests(&source), [r#"create Mara character ["a.png", "b.png"]"#]);

        let mut form = super::super::form::CreateForm::default();
        assert!(form.add_character_set(&images).is_none());
        form.set_saved_element(saved, &images);
        let expected = SavedElement {
            id: "el-new".to_owned(),
            owner: Some("user_1".to_owned()),
        };
        assert_eq!(form.saved_element(0, Some("user_1")), Some(&expected));
        assert_eq!(form.saved_element(0, Some("someone else")), None);
    }

    #[test]
    fn a_failed_save_speaks_in_plain_words() {
        let refused = SavedElement::failure(&PortError::Provider(ProviderError::Validation {
            field: None,
            message: "Image failed moderation".to_owned(),
        }));
        assert_eq!(
            refused,
            Some(UserMessage::new(
                "Can't save to Higgsfield",
                "Image failed moderation",
                None
            ))
        );
        assert!(
            SavedElement::failure(&PortError::Provider(ProviderError::Unauthorized))
                .and_then(|message| message.action)
                .is_some()
        );
        assert_eq!(SavedElement::failure(&PortError::Cancelled), None);
        assert_eq!(
            SavedElement::name(&format!("  {}", "a".repeat(40))),
            "a".repeat(32)
        );
    }

    #[test]
    fn picking_adds_what_fits_and_asks_when_over_the_limit() {
        let images = |count: usize| (0..count).map(image).collect::<Vec<_>>();
        let of = |count: usize| element("el", "Mara", None, images(count));
        assert_eq!(ElementPick::of(&of(3), 3), ElementPick::Add(images(3)));
        assert_eq!(ElementPick::of(&of(5), 2), ElementPick::Choose(images(5), 2));
        assert_eq!(
            ElementPick::of(&of(0), 2),
            ElementPick::Unavailable("This element has no images to use.".to_owned())
        );
        assert_eq!(
            ElementPick::of(&of(1), 0),
            ElementPick::Unavailable("References are full.".to_owned())
        );
    }
}
