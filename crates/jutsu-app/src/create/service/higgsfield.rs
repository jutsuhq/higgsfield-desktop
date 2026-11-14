//! References › From Higgsfield (`HiggsfieldReferencesSheet`, `UploadsPicker`, `ElementsPicker` and
//! `AppEnvironment+MotionLibrary.swift`): the account's uploads and elements, downloaded once under
//! `account-media/` and added as references.

use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt as _;
use jutsu_core::{
    AccountElement, CharacterSheet, ElementCategory, LocalImage, MediaLinkKind, ProviderKind, ServerFileName,
    ServerMedia, UploadKind, UploadedFile,
};
use jutsu_engine::MediaLinkDownloader;
use url::Url;

use super::{CreateError, CreateService, file_url};
use crate::connection::UserMessage;
use crate::create::account_references::{
    ElementOpening, HiggsfieldReferencesView, HiggsfieldSource, HiggsfieldSourceChoice, character_sheets,
    characters_panel, choosing, elements_panel, limit_count, uploads_panel,
};
use crate::create::elements::ElementPick;
use crate::create::form::mode_title;
use crate::create::paged_feed::{Fetch, PagedFeed};
use crate::images::generator::GeneratedImage;

/// What the sheet holds between calls.
#[derive(Debug, Default)]
pub(in crate::create) struct ReferencesState {
    uploads: Option<PagedFeed<UploadedFile>>,
    /// The account's character sheets; `None` without an account that makes them.
    characters: Option<PagedFeed<CharacterSheet>>,
    /// The element whose images are being picked.
    opened: Option<AccountElement>,
    /// What is being downloaded: `"uploads"` or an element's ID.
    adding: Option<String>,
    /// Counts the sheet's closings: a download that outlives its sheet adds nothing.
    session: u64,
}

impl ReferencesState {
    pub(in crate::create) fn new(account: &crate::provider::CreateAccount) -> Self {
        let characters = account.characters.clone().map(|maker| {
            let fetch: Fetch<CharacterSheet> = Arc::new(move |cursor| {
                let maker = maker.clone();
                async move {
                    let page = maker.character_sheets(cursor).await?;
                    Ok((page.items, page.next_cursor))
                }
                .boxed()
            });
            PagedFeed::new("Can't load your characters", fetch)
        });
        let uploads = account.uploads.clone().map(|source| {
            let fetch: Fetch<UploadedFile> = Arc::new(move |cursor| {
                let source = source.clone();
                async move {
                    let page = source.uploads(UploadKind::Image, cursor).await?;
                    Ok((page.items, page.next_cursor))
                }
                .boxed()
            });
            PagedFeed::new("Can't load your uploads", fetch)
        });
        Self {
            uploads,
            characters,
            ..Self::default()
        }
    }
}

const UPLOADS: &str = "uploads";

impl CreateService {
    /// The sheet; `None` without an account connection.
    pub fn higgsfield_references(&self) -> Option<HiggsfieldReferencesView> {
        let state = self.locked();
        let feed = state.account.references.uploads.as_ref()?;
        let limit = state.form.reference_room();
        let elements = state
            .account
            .elements
            .as_ref()
            .map(|model| elements_panel(model, limit));
        let mut sources = vec![HiggsfieldSourceChoice {
            value: HiggsfieldSource::Uploads,
            title: "Uploads".to_owned(),
        }];
        if elements.is_some() {
            sources.push(HiggsfieldSourceChoice {
                value: HiggsfieldSource::Elements,
                title: "Elements".to_owned(),
            });
        }
        let characters = state.account.services.characters.as_ref().map(|maker| {
            let owner = maker.owner();
            let (made, pending) = self.characters.get().map_or((Vec::new(), 0), |builder| {
                (
                    builder.log().sheets(owner.as_deref()),
                    builder.pending_count(owner.as_deref()),
                )
            });
            let account = state.account.references.characters.as_ref().map(PagedFeed::view);
            characters_panel(made, account.as_ref(), pending, limit)
        });
        if characters.is_some() {
            sources.push(HiggsfieldSourceChoice {
                value: HiggsfieldSource::Characters,
                title: "Characters".to_owned(),
            });
        }
        Some(HiggsfieldReferencesView {
            sources,
            limit: limit_count(limit),
            uploads: uploads_panel(&feed.view(), limit),
            elements,
            characters,
            adding: state.account.references.adding.clone(),
        })
    }

    /// Loads a tab's first page, or its next with `more`. Opened again later, the uploads show what was uploaded
    /// since.
    pub async fn load_higgsfield(self: &Arc<Self>, source: HiggsfieldSource, more: bool) {
        let (uploads, elements, characters) = {
            let state = self.locked();
            (
                state.account.references.uploads.clone(),
                state.account.elements.clone(),
                state.account.references.characters.clone(),
            )
        };
        match source {
            HiggsfieldSource::Uploads => {
                let Some(feed) = uploads else { return };
                if more {
                    feed.load_more().await;
                } else if feed.view().has_loaded {
                    feed.refresh().await;
                } else {
                    feed.load_if_needed().await;
                }
            }
            HiggsfieldSource::Elements => {
                let Some(model) = elements else { return };
                if more {
                    model.load_more().await;
                } else {
                    model.load_if_needed().await;
                }
            }
            HiggsfieldSource::Characters => {
                let Some(feed) = characters else { return };
                if more {
                    feed.load_more().await;
                } else {
                    feed.load_if_needed().await;
                }
            }
        }
        self.notify();
    }

    /// The Category menu of the Elements tab; `None` shows every category.
    pub fn set_element_category(&self, category: Option<&str>) -> Result<(), CreateError> {
        let category = category
            .map(|raw| {
                ElementCategory::from_raw(raw)
                    .ok_or_else(|| CreateError::Invalid(format!("Unknown category “{raw}”.")))
            })
            .transpose()?;
        if let Some(model) = self.locked().account.elements.clone() {
            model.set_category(category);
        }
        self.notify();
        Ok(())
    }

    /// The sheet closed: a download still running adds nothing.
    pub fn close_higgsfield(&self) {
        {
            let mut state = self.locked();
            let references = &mut state.account.references;
            references.session += 1;
            references.opened = None;
            references.adding = None;
        }
        self.notify();
    }

    /// Marks a download as running; `None` while another runs.
    fn start_adding(&self, what: &str) -> Option<u64> {
        let mut state = self.locked();
        let references = &mut state.account.references;
        if references.adding.is_some() {
            return None;
        }
        references.adding = Some(what.to_owned());
        Some(references.session)
    }

    /// Ends a download; adds `images` unless the sheet closed meanwhile. Answers whether they were added.
    fn finish_adding(self: &Arc<Self>, session: u64, images: Option<Vec<LocalImage>>) -> bool {
        let added = {
            let mut state = self.locked();
            if state.account.references.session != session {
                return false;
            }
            state.account.references.adding = None;
            match images {
                Some(images) => {
                    state.form.add_references(images.clone());
                    state.account.references.opened = None;
                    Some(images)
                }
                None => None,
            }
        };
        if let Some(images) = added {
            self.edited();
            self.make_thumbnails(images);
            true
        } else {
            self.notify();
            false
        }
    }

    /// Downloads the picked uploads once each and adds them in the order picked. Answers why it failed; `None`
    /// once they are added, or when the sheet closed first.
    pub async fn add_uploads(self: &Arc<Self>, ids: &[String]) -> Result<Option<String>, CreateError> {
        let Some(feed) = self.locked().account.references.uploads.clone() else {
            return Ok(None);
        };
        let items = feed.view().items;
        let uploads: Vec<UploadedFile> = ids
            .iter()
            .filter_map(|id| items.iter().find(|upload| &upload.id == id).cloned())
            .collect();
        let Some(session) = self.start_adding(UPLOADS) else {
            return Ok(None);
        };
        self.notify();
        let folder = self.deps.layout.account_media_folder();
        let mut images = Vec::with_capacity(uploads.len());
        for upload in &uploads {
            let Some(file) = download(&folder, std::slice::from_ref(&upload.id), &upload.url).await else {
                self.finish_adding(session, None);
                return Ok(Some("Can't download the images".to_owned()));
            };
            images.push(LocalImage::new(file_url(&file)?));
        }
        self.finish_adding(session, Some(images));
        Ok(None)
    }

    /// Picks an element: adds its images when they all fit, else asks which.
    pub async fn open_element(self: &Arc<Self>, id: &str) -> Result<ElementOpening, CreateError> {
        let Some(model) = self.locked().account.elements.clone() else {
            return Ok(ElementOpening::Failed {
                message: String::new(),
            });
        };
        let Some(element) = model.current().into_iter().find(|element| element.id == id) else {
            return Err(CreateError::NotFound(
                "This element is no longer on the account.".to_owned(),
            ));
        };
        let Some(session) = self.start_adding(id) else {
            return Ok(ElementOpening::Failed {
                message: String::new(),
            });
        };
        self.notify();
        let full = model.with_images(element).await;
        let limit = self.locked().form.reference_room();
        let full = match full {
            Ok(full) => full,
            Err(error) => {
                self.finish_adding(session, None);
                let message = UserMessage::from_error(&error, Some(ProviderKind::Account))
                    .map_or_else(|| "Can't open this element".to_owned(), |message| message.title);
                return Ok(ElementOpening::Failed { message });
            }
        };
        match ElementPick::of(&full, limit) {
            ElementPick::Add(images) => {
                let indices: Vec<usize> = (0..images.len()).collect();
                self.locked().account.references.adding = None;
                self.download_element(&full, &indices, session).await
            }
            ElementPick::Choose(_, limit) => {
                let opening = choosing(&full, limit);
                let mut state = self.locked();
                state.account.references.adding = None;
                state.account.references.opened = Some(full);
                drop(state);
                self.notify();
                Ok(opening)
            }
            ElementPick::Unavailable(message) => {
                self.finish_adding(session, None);
                Ok(ElementOpening::Failed { message })
            }
        }
    }

    /// Back from an element's images to the list.
    pub fn close_element(&self) {
        self.locked().account.references.opened = None;
        self.notify();
    }

    /// Adds the images picked of the element being chosen from, in the order picked.
    pub async fn add_element_images(
        self: &Arc<Self>,
        indices: &[usize],
    ) -> Result<ElementOpening, CreateError> {
        let (element, session) = {
            let state = self.locked();
            (
                state.account.references.opened.clone(),
                state.account.references.session,
            )
        };
        let Some(element) = element else {
            return Ok(ElementOpening::Failed {
                message: String::new(),
            });
        };
        self.download_element(&element, indices, session).await
    }

    /// Downloads the images at `indices` of `element` once each, keeping their media so a job of the same account
    /// names them instead of uploading the files.
    async fn download_element(
        self: &Arc<Self>,
        element: &AccountElement,
        indices: &[usize],
        session: u64,
    ) -> Result<ElementOpening, CreateError> {
        {
            let mut state = self.locked();
            let references = &mut state.account.references;
            if references.session != session || references.adding.is_some() {
                return Ok(ElementOpening::Failed {
                    message: String::new(),
                });
            }
            references.adding = Some(element.id.clone());
        }
        self.notify();
        let folder = self.deps.layout.account_media_folder();
        let mut images = Vec::with_capacity(indices.len());
        for &index in indices {
            let Some(image) = element.images.get(index) else {
                continue;
            };
            let name = image
                .media
                .as_ref()
                .map_or_else(|| format!("image-{}", index + 1), |media| media.id.clone());
            let names = ["elements".to_owned(), element.id.clone(), name];
            let Some(file) = download(&folder, &names, &image.url).await else {
                self.finish_adding(session, None);
                return Ok(ElementOpening::Failed {
                    message: "Can't download the images".to_owned(),
                });
            };
            let mut local = LocalImage::new(file_url(&file)?);
            local.server_media.clone_from(&image.media);
            images.push(local);
        }
        if self.finish_adding(session, Some(images)) {
            Ok(ElementOpening::Added)
        } else {
            Ok(ElementOpening::Failed {
                message: String::new(),
            })
        }
    }
}

impl CreateService {
    /// Downloads image `index` of `element` as From Higgsfield adds it, keeping its media so a job of the same
    /// account names it instead of uploading the file (`downloadElementImage`, for agents); `None` when it can't.
    pub async fn agent_element_image(&self, element: &AccountElement, index: usize) -> Option<LocalImage> {
        let image = element.images.get(index)?;
        let name = image
            .media
            .as_ref()
            .map_or_else(|| format!("image-{}", index + 1), |media| media.id.clone());
        let names = ["elements".to_owned(), element.id.clone(), name];
        let file = download(&self.deps.layout.account_media_folder(), &names, &image.url).await?;
        let mut local = LocalImage::new(file_url(&file).ok()?);
        local.server_media.clone_from(&image.media);
        Some(local)
    }

    /// Create Character's builder, whose log lists the sheets made here. Set once, at launch.
    pub fn use_characters(&self, builder: Arc<crate::images::characters::CharacterBuilder>) {
        let _ = self.characters.set(builder);
    }

    /// Picks a sheet of the Characters tab: adds it as one reference image, which a job of the same account names by
    /// its job ID. Answers why it failed; `None` once it is added, or when the sheet closed first.
    pub async fn add_character(self: &Arc<Self>, id: &str) -> Result<Option<String>, CreateError> {
        let (maker, account) = {
            let state = self.locked();
            (
                state.account.services.characters.clone(),
                state.account.references.characters.as_ref().map(PagedFeed::view),
            )
        };
        let Some(maker) = maker else {
            return Ok(None);
        };
        let owner = maker.owner();
        let made = self
            .characters
            .get()
            .map(|builder| builder.log().sheets(owner.as_deref()))
            .unwrap_or_default();
        let Some(sheet) = character_sheets(made, account.as_ref())
            .into_iter()
            .find(|sheet| sheet.id == id)
        else {
            return Err(CreateError::NotFound(
                "This character sheet is no longer listed.".to_owned(),
            ));
        };
        let Some(session) = self.start_adding(id) else {
            return Ok(None);
        };
        self.notify();
        let media = ServerMedia {
            id: sheet.id.clone(),
            url: sheet.image_url.clone(),
            owner,
        };
        // Saved in Library › Images already: the reference is that copy, so it isn't downloaded again.
        let saved = sheet
            .file_url
            .as_ref()
            .filter(|file| file.to_file_path().is_ok_and(|path| path.exists()))
            .cloned();
        let file = if let Some(file) = saved {
            file
        } else {
            let folder = self.deps.layout.account_media_folder();
            let names = ["characters".to_owned(), sheet.id.clone()];
            let Some(path) = download(&folder, &names, &sheet.image_url).await else {
                self.finish_adding(session, None);
                return Ok(Some("Can't download the sheet".to_owned()));
            };
            file_url(&path)?
        };
        let mut image = LocalImage::new(file);
        image.server_media = Some(media);
        self.finish_adding(session, Some(vec![image]));
        Ok(None)
    }

    /// How many more references the form takes: Generate…'s room for picks.
    pub fn reference_room(&self) -> usize {
        self.locked().form.reference_room()
    }

    /// Adds an image Generate… made like an opened file: its saved copy in Library › Images, or a download under
    /// `generated/` (cleared with the cache). Answers why it wasn't added; `None` once it is.
    pub async fn add_generated(
        self: &Arc<Self>,
        generated: &GeneratedImage,
    ) -> Result<Option<String>, CreateError> {
        // Saved in Library › Images already: the reference is that file, so it isn't downloaded twice.
        let saved = generated.file.clone().filter(|file| file.exists());
        let path = if let Some(file) = saved {
            file
        } else {
            let folder = self.deps.layout.generated_images_folder();
            let names = [generated.id.to_string()];
            let Some(path) = download(&folder, &names, &generated.url).await else {
                return Ok(Some("Can't download the image".to_owned()));
            };
            path
        };
        let mut image = LocalImage::new(file_url(&path)?);
        image.remote_origin = Some(generated.url.clone());
        image.prompt = Some(generated.prompt.clone());
        let full = {
            let mut state = self.locked();
            let added = state.form.add_references(vec![image.clone()]);
            (added == 0 && !state.form.can_add_references()).then(|| {
                format!(
                    "1 image wasn't added: {} takes up to {} reference images.",
                    mode_title(state.form.mode()),
                    state.form.limits().reference_images.upper()
                )
            })
        };
        if full.is_some() {
            return Ok(full);
        }
        self.edited();
        self.make_thumbnails(vec![image]);
        Ok(None)
    }
}

/// `link` as `folder/<names>.<ext>`, downloaded once; `None` when it can't be.
async fn download(folder: &std::path::Path, names: &[String], link: &Url) -> Option<PathBuf> {
    let file = ServerFileName::file(folder, names, link, &ServerFileName::IMAGE_EXTENSIONS)?;
    match MediaLinkDownloader::default()
        .download_to(link, &file, MediaLinkKind::Image)
        .await
    {
        Ok(()) => Some(file),
        Err(error) => {
            tracing::warn!(%error, "can't download an account image");
            None
        }
    }
}
