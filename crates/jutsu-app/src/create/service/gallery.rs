//! What Create shows from the connection beside the form (`CreateLoader.refreshPresets`, `AppEnvironment+Presets`,
//! `AppEnvironment+MotionLibrary`, `CreateView`): the style gallery, the Motion Library on the empty stage, batch
//! characters and saving them to the account.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt as _;
use jutsu_core::{LocalImage, MediaLinkKind, Mode, MotionShelf, PortError, RemoteAsset, ServerFileName};
use jutsu_engine::MediaLinkDownloader;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::{CreateError, CreateService, State, file_url};
use crate::connection::UserMessage;
use crate::create::constraints::{ModelConstraintCache, ModelConstraintStore};
use crate::create::elements::{AccountElementsModel, SavedElement};
use crate::create::form::{CreateBatch, Notice};
use crate::create::motion::{MotionLibraryModel, MotionLibraryView};
use crate::create::presets::{self, PresetCatalog, PresetsView};
use crate::create::view::{CreateSnapshot, SaveState};
use crate::preferences::Preferences;
use crate::provider::CreateAccount;

/// The style catalog, read once, and the provider generation it was last refreshed for.
#[derive(Debug, Default)]
pub(super) struct GalleryState {
    pub(super) catalog: Option<PresetCatalog>,
    pub(super) refreshed_for: Option<u64>,
    /// The model limits of the account, made on first use.
    pub(super) constraints: Option<Arc<ModelConstraintStore>>,
}

/// The connection's account services and what Create holds of them.
#[derive(Debug, Default)]
pub(super) struct AccountState {
    pub(super) services: CreateAccount,
    pub(super) motion: Option<Arc<MotionLibraryModel>>,
    pub(super) elements: Option<Arc<AccountElementsModel>>,
    /// The library clip being downloaded to become the source.
    pub(super) downloading_clip: Option<String>,
    /// Character sets being saved, by their files.
    pub(super) saving_sets: HashSet<Vec<Url>>,
    /// References › From Higgsfield.
    pub(super) references: super::higgsfield::ReferencesState,
}

/// Fills the parts of the snapshot that come from the gallery and the account.
pub(super) fn fill(view: &mut CreateSnapshot, state: &State, preferences: &Preferences) {
    let form = &state.form;
    if form.mode() == Mode::Restyle {
        let favorites = presets::decode(preferences.favorite_presets.as_ref());
        let recents = presets::decode(preferences.recent_presets.as_ref());
        let selection: Vec<Uuid> = if form.is_batch() {
            form.batch_preset_ids().to_vec()
        } else {
            form.preset_id().into_iter().collect()
        };
        let batch = form
            .is_batch()
            .then(|| (form.batch_count_text(), form.batch_cap_notice()));
        view.presets = Some(PresetsView::new(
            state.gallery.catalog.as_ref(),
            &favorites,
            &recents,
            &selection,
            batch,
        ));
    }
    view.shows_motion_library = state.account.motion.is_some() && form.mode() != Mode::Restyle;
    view.offers_higgsfield_references = state.account.services.uploads.is_some();
    if let Some(sets) = &mut view.character_sets
        && let Some(elements) = state
            .account
            .elements
            .as_ref()
            .filter(|elements| elements.can_publish())
    {
        let owner = elements.owner();
        for (index, set) in sets.sets.iter_mut().enumerate() {
            let key = form
                .character_sets()
                .get(index)
                .map(|images| CreateBatch::key(images))
                .unwrap_or_default();
            set.save = if form.saved_element(index, owner.as_deref()).is_some() {
                SaveState::Saved
            } else if state.account.saving_sets.contains(&key) {
                SaveState::Saving
            } else {
                SaveState::Available
            };
        }
    }
}

impl CreateService {
    /// The account services of a new connection (`None`s for an API key or while disconnected): the Motion Library
    /// and the elements start over.
    pub async fn set_account(&self, account: CreateAccount) {
        {
            let mut state = self.locked();
            state.account = AccountState {
                references: super::higgsfield::ReferencesState::new(&account),
                motion: account
                    .motion
                    .clone()
                    .map(|library| Arc::new(MotionLibraryModel::new(&library))),
                elements: account
                    .elements
                    .clone()
                    .map(|elements| Arc::new(AccountElementsModel::new(elements, true))),
                services: account,
                downloading_clip: None,
                saving_sets: HashSet::new(),
            };
        }
        self.switch_constraints().await;
        self.notify();
    }

    // MARK: Model limits

    fn constraint_store(&self) -> Arc<ModelConstraintStore> {
        let folder = self.deps.layout.cache_folder();
        self.locked()
            .gallery
            .constraints
            .get_or_insert_with(|| {
                Arc::new(ModelConstraintStore::new(
                    ModelConstraintCache::new(&folder),
                    OffsetDateTime::now_utc,
                ))
            })
            .clone()
    }

    /// Shows the new account's cached limits, or the constants for a connection that can't report them.
    async fn switch_constraints(&self) {
        let account = self
            .locked()
            .account
            .services
            .constraints
            .is_some()
            .then(|| self.deps.provider.current().and_then(|provider| provider.owner()))
            .flatten();
        let store = self.constraint_store();
        store.switch_account(account.as_deref()).await;
        self.use_constraints(store.table());
    }

    fn use_constraints(&self, table: Option<jutsu_core::ModelConstraintTable>) {
        self.locked().form.set_constraints(table);
    }

    /// The limits the provider checks a request against: the connected account's cached table, else the built-in
    /// one. The form shows the same table.
    pub fn model_constraints(&self) -> jutsu_core::ModelConstraintTable {
        self.locked()
            .gallery
            .constraints
            .as_ref()
            .and_then(|store| store.table())
            .unwrap_or_else(jutsu_core::ModelConstraintTable::built_in)
    }

    /// Reads the account's limits from the server when its table is a day old or missing.
    pub(super) fn ensure_constraints(self: &Arc<Self>) {
        let store = self.constraint_store();
        let Some(source) = self.locked().account.services.constraints.clone() else {
            return;
        };
        if !store.is_stale() {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            let changed = store
                .refresh_if_stale(move || async move { source.model_constraints().await }.boxed())
                .await;
            if changed {
                service.use_constraints(store.table());
                service.edited();
            }
        });
    }

    // MARK: Styles

    /// Shows the cached catalog the first time, and refreshes it once per provider.
    pub(super) fn ensure_presets(self: &Arc<Self>) {
        let generation = self.deps.provider.generation();
        let connected = self.deps.provider.current().is_some();
        let refresh = {
            let mut state = self.locked();
            if state.gallery.catalog.is_none() {
                let mut catalog = PresetCatalog::new(self.deps.layout.preset_cache_folder());
                catalog.load_cache();
                state.gallery.catalog = Some(catalog);
            }
            let due = connected && state.gallery.refreshed_for != Some(generation);
            if due {
                state.gallery.refreshed_for = Some(generation);
            }
            due
        };
        if refresh {
            let service = self.clone();
            tokio::spawn(async move { service.refresh_presets().await });
        }
    }

    /// Fetches the catalog and caches it; ignored while a refresh runs or without a connection.
    pub async fn refresh_presets(self: &Arc<Self>) {
        let Some(provider) = self.deps.provider.current() else {
            return;
        };
        let started = {
            let mut state = self.locked();
            let folder = self.deps.layout.preset_cache_folder();
            state
                .gallery
                .catalog
                .get_or_insert_with(|| PresetCatalog::new(folder))
                .begin_refresh()
        };
        if !started {
            return;
        }
        self.notify();
        let fetched = provider.presets().await;
        if let Some(catalog) = &mut self.locked().gallery.catalog {
            catalog.finish_refresh(fetched, Some(provider.kind()));
        }
        self.notify();
    }

    /// The gallery's search.
    pub async fn set_preset_query(self: &Arc<Self>, query: String) -> CreateSnapshot {
        if let Some(catalog) = &mut self.locked().gallery.catalog {
            catalog.query = query;
        }
        self.notify();
        self.snapshot().await
    }

    /// Stars a catalog style as the newest favorite, or unstars one.
    pub fn toggle_favorite_preset(&self, id: Uuid) -> Result<(), CreateError> {
        let catalog = self
            .locked()
            .gallery
            .catalog
            .as_ref()
            .map(|catalog| catalog.presets.clone())
            .unwrap_or_default();
        self.deps
            .preferences
            .edit(|preferences| {
                let favorites = presets::decode(preferences.favorite_presets.as_ref());
                preferences.favorite_presets = Some(presets::encode(&presets::toggled_favorite(
                    id, &favorites, &catalog,
                )));
            })
            .map_err(|error| CreateError::Invalid(error.to_string()))?;
        self.notify();
        Ok(())
    }

    /// Puts a queued job's style on top of the gallery's Recent section.
    pub(super) fn record_preset_use(&self, id: Uuid) {
        let cached: Vec<_> = presets::cached_preset(id, &self.deps.layout.preset_cache_folder())
            .into_iter()
            .collect();
        let edited = self.deps.preferences.edit(|preferences| {
            let recents = presets::recording_use(
                id,
                &presets::decode(preferences.recent_presets.as_ref()),
                &cached,
                &presets::decode(preferences.favorite_presets.as_ref()),
            );
            preferences.recent_presets = Some(presets::encode(&recents));
        });
        if let Err(error) = edited {
            tracing::error!(%error, "can't record the style as recent");
        }
    }

    // MARK: Motion Library

    /// The browser on the empty stage; `None` when the connection has no library.
    pub fn motion_library(&self) -> Option<MotionLibraryView> {
        let (model, downloading) = {
            let state = self.locked();
            (
                state.account.motion.clone()?,
                state.account.downloading_clip.clone(),
            )
        };
        Some(model.view(downloading.as_deref()))
    }

    /// Shows `shelf` (or the shelf shown) and loads its first page, or its next page with `more`.
    pub async fn load_motion_library(
        self: &Arc<Self>,
        named: Option<&str>,
        more: bool,
    ) -> Result<(), CreateError> {
        let picked = named
            .map(|raw| {
                MotionShelf::from_raw(raw)
                    .ok_or_else(|| CreateError::Invalid(format!("Unknown shelf “{raw}”.")))
            })
            .transpose()?;
        let Some(model) = self.locked().account.motion.clone() else {
            return Ok(());
        };
        if let Some(picked) = picked {
            model.set_shelf(picked);
        }
        self.notify();
        if more {
            model.load_more().await;
        } else {
            model.load_if_needed().await;
        }
        self.notify();
        Ok(())
    }

    /// Downloads a library clip once, then opens it as the source like a picked file.
    pub async fn use_motion_clip(self: &Arc<Self>, id: &str) -> Result<(), CreateError> {
        let preset = {
            let mut state = self.locked();
            if state.account.downloading_clip.is_some() {
                return Ok(());
            }
            let Some(model) = state.account.motion.clone() else {
                return Ok(());
            };
            let preset = model
                .current()
                .into_iter()
                .find(|preset| preset.id == id)
                .ok_or_else(|| CreateError::NotFound("This clip is no longer in the library.".to_owned()))?;
            state.account.downloading_clip = Some(preset.id.clone());
            preset
        };
        self.notify();
        let file = ServerFileName::file(
            &self.deps.layout.motion_library_folder(),
            &[preset.id.clone(), preset.name.clone()],
            &preset.video_url,
            &ServerFileName::VIDEO_EXTENSIONS,
        );
        let downloaded = match file {
            Some(file) => MediaLinkDownloader::default()
                .download_to(&preset.video_url, &file, MediaLinkKind::Video)
                .await
                .map(|()| file)
                .map_err(|error| error.to_string()),
            None => Err("The file name is invalid.".to_owned()),
        };
        self.locked().account.downloading_clip = None;
        self.notify();
        let file = downloaded.map_err(|error| {
            tracing::warn!(%error, "can't download a Motion Library clip");
            CreateError::Media("Check your connection and try again.".to_owned())
        })?;
        self.open_source(&file).await
    }

    // MARK: Batch characters

    /// Picked or dropped images as one new batch character; says so when the batch is full or images didn't fit.
    pub fn add_character_set(self: &Arc<Self>, paths: &[PathBuf]) -> Result<Option<Notice>, CreateError> {
        let images: Vec<LocalImage> = paths
            .iter()
            .filter(|path| jutsu_media::frames::is_image(path))
            .map(|path| file_url(path).map(LocalImage::new))
            .collect::<Result<_, _>>()?;
        if images.is_empty() {
            return Ok(None);
        }
        let notice = self.locked().form.add_character_set(&images);
        self.edited();
        self.make_thumbnails(images);
        Ok(notice)
    }

    /// "Save to Higgsfield…": uploads the set at `index` and publishes it as a character element named `name`, then
    /// records it on the draft. Answers why it failed.
    pub async fn save_character_set(
        self: &Arc<Self>,
        index: usize,
        name: &str,
    ) -> Result<Option<UserMessage>, CreateError> {
        let (elements, images) = {
            let mut state = self.locked();
            let elements = state
                .account
                .elements
                .clone()
                .filter(|elements| elements.can_publish());
            let images = state.form.character_sets().get(index).cloned();
            let (Some(elements), Some(images)) = (elements, images) else {
                return Ok(None);
            };
            if !state.account.saving_sets.insert(CreateBatch::key(&images)) {
                return Ok(None);
            }
            (elements, images)
        };
        let Some(provider) = self.deps.provider.current() else {
            self.locked()
                .account
                .saving_sets
                .remove(&CreateBatch::key(&images));
            return Ok(None);
        };
        self.notify();
        let preparer = self.deps.preparer.clone();
        let published = elements
            .publish(&images, name, |image| {
                let (preparer, provider) = (preparer.clone(), provider.clone());
                async move { provider.upload(preparer.prepare_image(&image).await?).await }
            })
            .await;
        self.locked()
            .account
            .saving_sets
            .remove(&CreateBatch::key(&images));
        match published {
            Ok(saved) => {
                self.locked().form.set_saved_element(saved, &images);
                self.edited();
                Ok(None)
            }
            Err(error) => {
                self.notify();
                Ok(SavedElement::failure(&error))
            }
        }
    }

    /// The account's reference elements, for agents (`AppEnvironment.accountElements`); `None` for an API key.
    pub fn agent_elements(&self) -> Option<Arc<dyn jutsu_core::AccountElements>> {
        self.locked().account.services.elements.clone()
    }

    /// Uploads an image as Save to Higgsfield does, for an agent's `create_element` (`elementUploader`); `None`
    /// without a connection.
    pub fn agent_element_uploader(&self) -> Option<ElementUploader> {
        let provider = self.deps.provider.current()?;
        let preparer = self.deps.preparer.clone();
        Some(Arc::new(move |image: LocalImage| {
            let (preparer, provider) = (preparer.clone(), provider.clone());
            Box::pin(async move { provider.upload(preparer.prepare_image(&image).await?).await })
        }))
    }
}

/// Uploads one image for an agent's `create_element`.
pub type ElementUploader = Arc<
    dyn Fn(LocalImage) -> futures::future::BoxFuture<'static, Result<RemoteAsset, PortError>> + Send + Sync,
>;
