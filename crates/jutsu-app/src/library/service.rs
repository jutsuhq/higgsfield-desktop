//! The Library screen (`LibraryView`, `AppEnvironment+History`, `AppEnvironment+Library`): the grid's snapshot read
//! from the catalog, a result's page, and the actions: star and like, delete, keep and reject with Undo, make
//! primary, the account's history and its downloads, and saving a result's inputs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use jutsu_core::{
    Culling, GenerationHistory, GenerationLikes, Job, Material, MaterialGeneration, MaterialKind,
    MaterialReference, MaterialRemote, MediaLinkKind, OutputSubfolder, PortError, ProviderError, SavedImage,
    ViralityScoring,
};
use jutsu_engine::{JobEngine, MediaLinkDownloader};
use jutsu_media::FfmpegTools;
use jutsu_store::{FileLayout, Store, StoreChange};
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

use super::actions::{
    LibraryStarring, StarError, StarStore, SystemTrash, Trash, deletion_message, earlier_files,
    image_deletion_message, like_not_saved, trash_files,
};
use super::chain::chain;
use super::culling::{CatalogIndex, CullingAction, CullingHistory};
use super::enhance::{EnhanceAvailability, EnhanceMenu, MadeFromLink, OFFERS_POST_OPERATIONS};
use super::filter::FilterOptions;
use super::history::{AccountHistory, HistoryPhase, HistoryPlaceholder, unavailable_reason};
use super::inputs::{InputFile, InputRole};
use super::item::LibraryItem;
use super::media::{KindCounts, LibraryKind, LibraryOrigin, MediaRow, in_origin, material_row};
use super::relations::{MaterialLink, MaterialRelations};
use super::thumbnail::{TILE_ASPECT, ThumbnailFit, cached_frame, write_frame};
use super::variants::VariantGroups;
use super::view::{
    CardKind, CardMenu, HistoryFooter, ImageDetails, InputView, LibraryCard, LibraryEmpty,
    LibraryEmptyAction, LibraryRequest, LibrarySnapshot, LinkView, MediaFile, ResultDetail, SourcePlayback,
};
use crate::connection::{ConnectionStore, UserMessage};
use crate::preferences::PreferencesStore;
use crate::provider::ProviderHandle;

/// Turns a file into the URL the window loads it from; `None` when it can't be shown.
pub type UrlFor<'a> = &'a (dyn Fn(&Path) -> Option<String> + Sync);

/// Why the Library can't answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LibraryError {
    /// The catalog can't be read.
    #[error("Can't open the material catalog: {0}")]
    Catalog(String),
    /// What was asked for isn't in the Library.
    #[error("{0}")]
    NotFound(String),
}

/// The account's history and likes of the active connection, with the provider generation they belong to.
#[derive(Clone, Default)]
pub struct AccountServices {
    pub generation: u64,
    pub history: Option<Arc<dyn GenerationHistory>>,
    pub likes: Option<Arc<dyn GenerationLikes>>,
    pub virality: Option<Arc<dyn ViralityScoring>>,
    /// The voices Change Voice offers.
    pub voices: Option<Arc<dyn jutsu_core::VoiceLibrary>>,
    /// The account's credit movements, for Settings › Spending.
    pub credits: Option<Arc<dyn jutsu_core::CreditHistory>>,
}

#[derive(Default)]
struct State {
    account: AccountServices,
    history: AccountHistory,
    culling: CullingHistory,
    downloading: HashSet<Uuid>,
    /// Results whose score is being asked for, and why the last scoring of a result gave nothing.
    scoring: HashSet<Uuid>,
    score_failures: HashMap<Uuid, String>,
    /// The provider generation whose presets were fetched for their names.
    presets_fetched: Option<u64>,
    /// Export for Editing runs; canceled through this.
    editing_export: Option<tokio_util::sync::CancellationToken>,
    /// The delivery set being exported, and its cancel.
    delivery_export: Option<(Uuid, tokio_util::sync::CancellationToken)>,
    /// Each file's state in the current or last delivery run, by task ID.
    delivery_states: HashMap<String, crate::library::delivery::DeliveryTaskState>,
    /// Settings › Spending's credit history.
    credit_feed: Option<spending::CreditFeed>,
}

pub struct LibraryService {
    store: Arc<Store>,
    layout: FileLayout,
    preferences: Arc<PreferencesStore>,
    connection: Arc<ConnectionStore>,
    provider: ProviderHandle,
    engine: JobEngine,
    utc_offset: UtcOffset,
    downloader: MediaLinkDownloader,
    tools: Option<FfmpegTools>,
    /// The image charges, virality scores included: written as each score is sent, counted toward the cap.
    image_spend: Arc<crate::images::spend_log::ImageSpendLog>,
    state: Mutex<State>,
    changes: watch::Sender<u64>,
    /// Enhance on this computer, for the menu's local items and to stop runs before a delete.
    on_device: std::sync::OnceLock<Arc<crate::on_device::service::OnDeviceService>>,
}

impl std::fmt::Debug for LibraryService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("LibraryService").finish_non_exhaustive()
    }
}

impl LibraryService {
    #[allow(
        clippy::too_many_arguments,
        reason = "the stores and services the Library reads and writes"
    )]
    pub fn new(
        store: Arc<Store>,
        layout: FileLayout,
        preferences: Arc<PreferencesStore>,
        connection: Arc<ConnectionStore>,
        provider: ProviderHandle,
        engine: JobEngine,
        utc_offset: UtcOffset,
        image_spend: Arc<crate::images::spend_log::ImageSpendLog>,
    ) -> Self {
        Self {
            store,
            layout,
            preferences,
            connection,
            provider,
            engine,
            utc_offset,
            image_spend,
            downloader: MediaLinkDownloader::default(),
            tools: FfmpegTools::from_environment(),
            state: Mutex::new(State::default()),
            changes: watch::channel(0).0,
            on_device: std::sync::OnceLock::new(),
        }
    }

    /// A revision that goes up whenever the catalog or anything else the Library shows changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn notify(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Follows the catalog's changes until the store closes.
    pub fn follow_store(self: &Arc<Self>) {
        let mut changes = self.store.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                match changes.recv().await {
                    Ok(StoreChange::Materials) | Err(RecvError::Lagged(_)) => {}
                    Ok(_) => continue,
                    Err(RecvError::Closed) => return,
                }
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.notify();
            }
        });
    }

    /// The history and likes of a new provider; the history starts over for another account.
    pub fn set_account(&self, services: AccountServices) {
        let owner = self.owner();
        {
            let mut state = self.locked();
            state.account = services;
            state.history.switch_account(owner);
        }
        self.notify();
    }

    fn owner(&self) -> Option<String> {
        self.provider.current().and_then(|provider| provider.owner())
    }

    fn starring(&self) -> LibraryStarring {
        LibraryStarring {
            likes: self.locked().account.likes.clone(),
            owner: self.owner(),
        }
    }

    async fn materials(&self) -> Result<Vec<Material>, LibraryError> {
        self.store.materials().await.map_err(|error| {
            tracing::error!(%error, "can't read the catalog");
            LibraryError::Catalog(error.to_string())
        })
    }

    async fn material(&self, id: Uuid) -> Result<Material, LibraryError> {
        self.store
            .material(id)
            .await
            .map_err(|error| LibraryError::Catalog(error.to_string()))?
            .ok_or_else(|| LibraryError::NotFound("This result is no longer in the library.".to_owned()))
    }

    /// The tab counts of this computer's Library, for the sidebar.
    pub async fn counts(&self) -> KindCounts {
        let Ok(materials) = self.materials().await else {
            return KindCounts::default();
        };
        let index = CatalogIndex::new(&materials);
        let rows: Vec<MediaRow> = materials
            .iter()
            .filter(|material| in_origin(material, LibraryOrigin::Local, None))
            .filter_map(material_row)
            .collect();
        let filter = super::filter::LibraryFilter::default();
        filter.counts(&filter.culled(rows, &index))
    }

    /// Everything the grid shows for `request`.
    pub async fn snapshot(
        &self,
        request: &LibraryRequest,
        now: OffsetDateTime,
        url_for: UrlFor<'_>,
    ) -> Result<LibrarySnapshot, LibraryError> {
        let materials = self.materials().await?;
        let owner = self.owner();
        let index = CatalogIndex::new(&materials);
        let mut rows: Vec<MediaRow> = materials
            .iter()
            .filter(|material| in_origin(material, request.origin, owner.as_deref()))
            .filter_map(material_row)
            .collect();
        self.name_presets(&mut rows).await;
        let by_id: HashMap<Uuid, &Material> =
            materials.iter().map(|material| (material.id, material)).collect();
        let is_origin_empty = rows.is_empty();
        let images_total = rows.iter().filter(|row| !row.is_video()).count();
        let options = FilterOptions::new(&rows);
        let filter = &request.filter;
        let culled = filter.culled(rows, &index);
        let counts = filter.counts(&culled);
        let shown = filter.apply(&culled, request.kind);
        let ids: Vec<Uuid> = shown.iter().map(MediaRow::id).collect();
        let groups = VariantGroups::new(&ids, &index);
        let expanded: HashSet<Uuid> = request
            .expanded_groups
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect();
        let starring = self.starring();
        let (connection, can_undo, downloading) = {
            let state = self.locked();
            (
                self.connection.state(),
                state.culling.can_undo(),
                state.downloading.clone(),
            )
        };
        let cards: Vec<LibraryCard> = shown
            .iter()
            .filter(|row| groups.shows(row.id(), &expanded))
            .map(|row| {
                let mut card = self.card(row, now, url_for, &starring);
                let id = row.id();
                card.is_downloading = downloading.contains(&id);
                card.culling = index.culling(id);
                card.variant_group = groups.group(id).map(|group| group.to_string());
                card.variant_count = groups
                    .collapsed_count(id, &expanded)
                    .map(|count| i32::try_from(count).unwrap_or(i32::MAX));
                card.is_variant_primary = index
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.is_variant_primary);
                match row {
                    MediaRow::Video(item) => card.menu = self.card_menu(item, by_id.get(&id).copied(), now),
                    // A saved image made from an earlier version offers Compare with Original while its file is here.
                    MediaRow::LocalImage(_) if !card.is_unavailable => {
                        card.menu.has_original = by_id
                            .get(&id)
                            .is_some_and(|material| follow_ups::original_version(material).is_some());
                    }
                    _ => {}
                }
                card
            })
            .collect();
        let cards = Self::fold_batches(cards, &shown, &groups, now);

        let placeholder = (request.origin == LibraryOrigin::Higgsfield)
            .then(|| HistoryPlaceholder::new(&connection, &self.locked().history, is_origin_empty))
            .flatten();
        let empty = (placeholder.is_none() && cards.is_empty())
            .then(|| grid_empty(request, is_origin_empty, images_total));
        let history = (request.origin == LibraryOrigin::Higgsfield).then(|| {
            let state = self.locked();
            let history = &state.history;
            HistoryFooter {
                is_loading: history.is_loading(),
                has_more: history.has_more(),
                failure: match history.phase() {
                    HistoryPhase::Failed(message) => Some(message.title.clone()),
                    _ => None,
                },
                is_waiting: history.is_waiting_to_load_more(),
                can_refresh: unavailable_reason(&connection).is_none(),
                note: history.footer_note().map(str::to_owned),
            }
        });
        Ok(LibrarySnapshot {
            fields: options.fields(request.kind, filter),
            options,
            cards,
            counts,
            is_filter_active: filter.is_active(),
            is_origin_empty,
            empty,
            placeholder,
            history,
            can_undo,
        })
    }

    fn card(
        &self,
        row: &MediaRow,
        now: OffsetDateTime,
        url_for: UrlFor<'_>,
        starring: &LibraryStarring,
    ) -> LibraryCard {
        match row {
            MediaRow::Video(item) => {
                let file = item.output_file();
                let is_unavailable = file.as_ref().is_some_and(|path| !path.exists());
                LibraryCard {
                    id: item.id().to_string(),
                    kind: CardKind::Video,
                    title: item.title(),
                    subtitle: item.subtitle(now),
                    accessibility_label: item.accessibility_label(now),
                    mode_tag: item.mode_tag().to_owned(),
                    is_draft: item.spec().is_draft,
                    duration_text: Some(item.duration_text()),
                    aspect_text: None,
                    prompt: item.spec().prompt.clone(),
                    is_starred: item.is_starred,
                    can_star: starring.can_star(item),
                    culling: Culling::Unreviewed,
                    media_url: file
                        .as_deref()
                        .filter(|_| !is_unavailable)
                        .and_then(url_for)
                        .or_else(|| item.remote_url().map(Url::to_string)),
                    file_path: file.map(|path| path.to_string_lossy().into_owned()),
                    needs_import: item.needs_import(),
                    is_downloading: false,
                    is_unavailable,
                    variant_group: None,
                    variant_count: None,
                    is_variant_primary: false,
                    fit: ThumbnailFit::new(item.aspect_ratio(), TILE_ASPECT),
                    request_id: item.job.remote.as_ref().map(|remote| remote.request_id.clone()),
                    menu: CardMenu::default(),
                    batch: None,
                    virality: item.job.virality.as_ref().and_then(super::virality::badge_text),
                }
            }
            MediaRow::LocalImage(image) => {
                let path = image.file_url.to_file_path().ok();
                let is_unavailable = path.as_ref().is_none_or(|path| !path.exists());
                let mut card = self.image_card(
                    row,
                    image.title.clone().unwrap_or_else(|| image.prompt.clone()),
                    &image.prompt,
                    path.as_deref().filter(|_| !is_unavailable).and_then(url_for),
                    path.map(|path| path.to_string_lossy().into_owned()),
                    is_unavailable,
                    true,
                );
                // A missing file offers only its prompt, its star and Remove from Library.
                card.menu.can_compare = !is_unavailable;
                card.menu.can_copy_image = !is_unavailable;
                card
            }
            MediaRow::RemoteImage(image) => {
                let can_star = starring.likes.is_some();
                // An image made without a prompt goes by its model (`RemoteImageCard`).
                let title = if image.prompt.is_empty() {
                    image.model.clone()
                } else {
                    image.prompt.clone()
                };
                let mut card = self.image_card(
                    row,
                    title,
                    &image.prompt,
                    Some(image.output_url.to_string()),
                    None,
                    false,
                    can_star,
                );
                card.needs_import = true;
                card.request_id = Some(image.id.clone());
                card.menu.can_compare = true;
                card
            }
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the card's parts that differ between local and remote images"
    )]
    fn image_card(
        &self,
        row: &MediaRow,
        title: String,
        prompt: &str,
        media_url: Option<String>,
        file_path: Option<String>,
        is_unavailable: bool,
        can_star: bool,
    ) -> LibraryCard {
        let model = row.image_model().unwrap_or_default().to_owned();
        let subtitle = [model.clone(), date_text(row.created_at(), self.utc_offset)]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let ratio = row.aspect_ratio().and_then(|aspect| {
            let (width, height) = aspect.split_once(':')?;
            Some(width.parse::<f64>().ok()? / height.parse::<f64>().ok()?)
        });
        let title = if title.trim().is_empty() {
            "No prompt".to_owned()
        } else {
            title
        };
        let mut label = vec![title.clone(), "Image".to_owned(), subtitle.clone()];
        if row.is_starred() {
            label.push("Starred".to_owned());
        }
        LibraryCard {
            id: row.id().to_string(),
            kind: CardKind::Image,
            accessibility_label: label.join(", "),
            title,
            subtitle,
            mode_tag: "Image".to_owned(),
            is_draft: false,
            duration_text: None,
            aspect_text: row.aspect_ratio(),
            prompt: prompt.to_owned(),
            is_starred: row.is_starred(),
            can_star,
            culling: Culling::Unreviewed,
            file_path,
            media_url,
            needs_import: false,
            is_downloading: false,
            is_unavailable,
            variant_group: None,
            variant_count: None,
            is_variant_primary: false,
            fit: ThumbnailFit::new(ratio, TILE_ASPECT),
            request_id: None,
            menu: CardMenu::default(),
            batch: None,
            virality: None,
        }
    }

    /// The cached frame of a video result, taken with ffmpeg the first time; `None` without ffmpeg or a file.
    pub async fn thumbnail(&self, id: Uuid) -> Result<Option<PathBuf>, LibraryError> {
        let material = self.material(id).await?;
        let Some(file) = material
            .current
            .file_url
            .as_ref()
            .and_then(|url| url.to_file_path().ok())
            .filter(|path| path.exists())
        else {
            return Ok(None);
        };
        if material.kind == MaterialKind::Image {
            return Ok(Some(file));
        }
        let Some(tools) = &self.tools else {
            return Ok(None);
        };
        let target = cached_frame(&self.layout.cache_folder(), material.current.id);
        match write_frame(tools, &file, &target).await {
            Ok(()) => Ok(Some(target)),
            Err(error) => {
                tracing::warn!(%error, "can't take a video's frame");
                Ok(None)
            }
        }
    }

    /// A result's page, by its material ID or the ID of the job that made it. `url_for` may make the source's
    /// folder loadable.
    pub async fn detail(
        &self,
        id: Uuid,
        now: OffsetDateTime,
        url_for: UrlFor<'_>,
    ) -> Result<ResultDetail, LibraryError> {
        let materials = self.materials().await?;
        let material = materials
            .iter()
            .find(|material| material.id == id)
            .or_else(|| {
                // A job whose result became a later version of another result opens that one.
                materials.iter().find(|material| {
                    material
                        .versions()
                        .iter()
                        .any(|version| version.video.as_ref().is_some_and(|job| job.id == id))
                })
            })
            .ok_or_else(|| LibraryError::NotFound("This result is no longer in the library.".to_owned()))?;
        let id = material.id;
        let Some(MediaRow::Video(mut item)) = material_row(material).or_else(|| {
            // A result taken off the Library still opens from a link to it.
            let mut listed = material.clone();
            listed.is_hidden = false;
            material_row(&listed)
        }) else {
            return Err(LibraryError::NotFound(
                "This result is no longer in the library.".to_owned(),
            ));
        };
        let items: Vec<LibraryItem> = materials
            .iter()
            .filter_map(material_row)
            .filter_map(|row| match row {
                MediaRow::Video(item) => Some(item),
                _ => None,
            })
            .collect();
        item.preset_name = self.preset_name(item.spec()).await;
        let relations = MaterialRelations::new(&materials, Path::exists);
        let spec = item.spec();
        let (source_line, source) = source_parts(spec.source.as_ref(), url_for);
        let base = item.source_name();
        let index = CatalogIndex::new(&materials);
        Ok(ResultDetail {
            id: id.to_string(),
            title: item.title(),
            headline: item.headline(now),
            prompt: spec.prompt.clone(),
            facts: item.facts(),
            range_text: item.range_text(),
            source_line,
            reference_previews: InputFile::all(&item)
                .iter()
                .filter(|input| matches!(input.role, InputRole::Reference(_)))
                .map(|input| {
                    input
                        .local
                        .as_deref()
                        .filter(|path| path.exists())
                        .and_then(url_for)
                        .or_else(|| input.remote.as_ref().map(Url::to_string))
                })
                .collect(),
            billing: item.billing_summary(),
            timing: item.timing_summary(),
            request_id: item.job.remote.as_ref().map(|remote| remote.request_id.clone()),
            correlation_id: item
                .job
                .remote
                .as_ref()
                .and_then(|remote| remote.correlation_id.clone()),
            is_starred: item.is_starred,
            star_title: if item.is_starred { "Unstar" } else { "Star" }.to_owned(),
            can_star: self.starring().can_star(&item),
            culling: index.culling(id),
            is_draft: spec.is_draft,
            result: media_file(item.output_file().as_deref(), url_for),
            remote_url: item.remote_url().map(Url::to_string),
            needs_import: item.needs_import(),
            source,
            aspect_ratio: item.aspect_ratio(),
            chain: chain(id, &items),
            made_from: relations
                .made_from(material.reference())
                .iter()
                .map(|link| link_view(link, url_for))
                .collect(),
            used_in: relations
                .used_in(id)
                .iter()
                .map(|link| link_view(link, url_for))
                .collect(),
            inputs: InputFile::all(&item)
                .iter()
                .map(|input| input_view(input, &base))
                .collect(),
            menu: self.card_menu(&item, Some(material), now),
            virality: self.virality_view(&item),
            enhance: self.enhance_menu(&item, id),
            operation_source: operation_source(&item, &items),
        })
    }

    /// The Enhance menu of a result's page.
    fn enhance_menu(&self, item: &LibraryItem, material_id: Uuid) -> Option<EnhanceMenu> {
        let availability =
            EnhanceAvailability::of(item, self.connection.state().kind(), OFFERS_POST_OPERATIONS);
        let Some(on_device) = self.on_device.get() else {
            return EnhanceMenu::new(&availability);
        };
        EnhanceMenu::with_local(&availability, on_device.offer(item), on_device.busy(material_id))
    }

    /// Hands the Library Enhance on this computer, once at launch.
    pub fn use_on_device(&self, on_device: Arc<crate::on_device::service::OnDeviceService>) {
        let _ = self.on_device.set(on_device);
    }

    /// An image's Details…: its prompt, what it was made from and the videos made from it.
    pub async fn image_details(
        &self,
        id: Uuid,
        version: Option<Uuid>,
        url_for: UrlFor<'_>,
    ) -> Result<ImageDetails, LibraryError> {
        let reference = MaterialReference::new(id, version.unwrap_or(Uuid::nil()));
        let materials = self.materials().await?;
        let relations = MaterialRelations::new(&materials, Path::exists);
        let material = materials
            .iter()
            .find(|material| material.id == reference.material_id)
            .ok_or_else(|| LibraryError::NotFound("This image is no longer in the library.".to_owned()))?;
        let version = relations.version(reference).unwrap_or(&material.current);
        let generation = version.generation.clone();
        let prompt = generation
            .as_ref()
            .map(|generation| generation.prompt.clone())
            .unwrap_or_default();
        let path = version.file_url.as_ref().and_then(|url| url.to_file_path().ok());
        Ok(ImageDetails {
            id: material.id.to_string(),
            reference: MaterialReference::new(material.id, version.id),
            has_prompt: !prompt.trim().is_empty(),
            prompt: if prompt.trim().is_empty() {
                "No prompt".to_owned()
            } else {
                prompt
            },
            model: generation.map(|generation| generation.model).unwrap_or_default(),
            created_text: date_text(version.created_at, self.utc_offset),
            image: media_file(path.as_deref(), url_for),
            remote_url: version.remote_url.as_ref().map(Url::to_string),
            is_starred: material.is_starred,
            made_from: relations
                .made_from(MaterialReference::new(material.id, version.id))
                .iter()
                .map(|link| link_view(link, url_for))
                .collect(),
            used_in: relations
                .used_in(material.id)
                .iter()
                .map(|link| link_view(link, url_for))
                .collect(),
        })
    }

    /// Stars or unstars the results and images of `ids`; on an account, likes them on Higgsfield to match. A refused
    /// like puts the star back and shows why.
    pub async fn set_starred(&self, ids: &[Uuid], is_starred: bool) {
        match self.star(ids, is_starred).await {
            Ok(()) => {}
            Err(StarError::Store(error)) => self.alert("Couldn't star the results", &error),
            Err(StarError::Like(error)) => self.connection.show_alert(like_not_saved(&error)),
        }
    }

    /// `set_starred` without the window's alert, for an agent's `library_mark`: the videos first, then the images
    /// only when every video's star was saved.
    pub async fn star(&self, ids: &[Uuid], is_starred: bool) -> Result<(), StarError> {
        let materials = self
            .materials()
            .await
            .map_err(|error| StarError::Store(error.to_string()))?;
        let wanted: HashSet<Uuid> = ids.iter().copied().collect();
        let rows: Vec<MediaRow> = materials
            .iter()
            .filter(|material| wanted.contains(&material.id))
            .filter_map(material_row)
            .collect();
        let starring = self.starring();
        let stars = StoreStars(self.store.clone());
        let mut confirmed = Vec::new();
        let videos: Vec<LibraryItem> = rows
            .iter()
            .filter_map(MediaRow::video)
            .filter(|item| starring.can_star(item))
            .cloned()
            .collect();
        let mut result = starring
            .set_all_starred(is_starred, &videos, &stars, &mut |id, is_liked| {
                confirmed.push((id.to_owned(), is_liked));
            })
            .await;
        if result.is_ok() {
            result = self
                .star_images(&rows, is_starred, &starring, &stars, &mut confirmed)
                .await;
        }
        {
            let mut state = self.locked();
            for (id, is_liked) in &confirmed {
                state.history.set_liked(id, *is_liked);
            }
        }
        if let Err(StarError::Like(error)) = &result {
            if *error == PortError::Provider(ProviderError::Unauthorized) {
                self.connection.mark_needs_attention();
            }
            tracing::error!(%error, "can't set a like");
        }
        result
    }

    async fn star_images(
        &self,
        rows: &[MediaRow],
        is_starred: bool,
        starring: &LibraryStarring,
        stars: &StoreStars,
        confirmed: &mut Vec<(String, bool)>,
    ) -> Result<(), StarError> {
        for row in rows
            .iter()
            .filter(|row| !row.is_video() && row.is_starred() != is_starred)
        {
            let like = match row {
                MediaRow::RemoteImage(image) => Some(image.id.clone()),
                _ => None,
            };
            match (like, &starring.likes) {
                (Some(id), Some(likes)) => {
                    likes
                        .set_liked(id.clone(), is_starred)
                        .await
                        .map_err(StarError::Like)?;
                    confirmed.push((id, is_starred));
                    stars
                        .set_starred(is_starred, row.id())
                        .await
                        .map_err(StarError::Store)?;
                }
                (Some(_), None) => {}
                (None, _) => stars
                    .set_starred(is_starred, row.id())
                    .await
                    .map_err(StarError::Store)?,
            }
        }
        Ok(())
    }

    /// Delete from Library: the files go to the trash with their earlier versions made here, then the results
    /// leave the Library. A result whose file couldn't be moved stays, and the alert says so.
    pub async fn delete(&self, ids: &[Uuid]) {
        self.delete_with(ids, &SystemTrash).await;
    }

    pub(crate) async fn delete_with(&self, ids: &[Uuid], trash: &dyn Trash) -> Option<UserMessage> {
        if let Some(on_device) = self.on_device.get() {
            on_device.runs().stop(ids).await;
        }
        let materials = match self.materials().await {
            Ok(materials) => materials,
            Err(error) => {
                let message = UserMessage::new(
                    "Can't remove the result from the Library",
                    error.to_string(),
                    None,
                );
                self.connection.show_alert(message.clone());
                return Some(message);
            }
        };
        let by_id: HashMap<Uuid, &Material> =
            materials.iter().map(|material| (material.id, material)).collect();
        // A result only on Higgsfield has no file or record to delete.
        let items: Vec<(Uuid, Option<PathBuf>)> = ids
            .iter()
            .filter_map(|id| by_id.get(id))
            .filter(|material| material.current.file_url.is_some())
            .map(|material| {
                (
                    material.id,
                    material
                        .current
                        .file_url
                        .as_ref()
                        .and_then(|url| url.to_file_path().ok()),
                )
            })
            .collect();
        let folders = self.output_folders();
        let owns = |path: &Path| folders.iter().any(|folder| path.starts_with(folder));
        let deletion = trash_files(&items, trash, |id| {
            by_id
                .get(&id)
                .map(|material| earlier_files(material, owns))
                .unwrap_or_default()
        });
        let saved = if deletion.deleted.is_empty() {
            Ok(())
        } else {
            self.store
                .remove_materials(&deletion.deleted)
                .await
                .map_err(|error| error.to_string())
        };
        let is_images = ids
            .iter()
            .filter_map(|id| by_id.get(id))
            .all(|material| material.kind == MaterialKind::Image);
        let message = if is_images {
            image_deletion_message(&deletion, saved)
        } else {
            deletion_message(&deletion, saved)
        };
        if let Some(message) = &message {
            self.connection.show_alert(message.clone());
        }
        message
    }

    /// Remove from Library for an image whose file is gone: only the listing goes.
    pub async fn remove(&self, ids: &[Uuid]) {
        if let Err(error) = self.store.remove_materials(ids).await {
            self.alert("Couldn't remove the images from the library", &error.to_string());
        }
    }

    /// Keep, reject or clear on `ids`; Keep or Reject on results that all have it already takes it back. Returns
    /// what was done, so the grid can move on.
    pub async fn cull(&self, action: CullingAction, ids: &[Uuid]) -> Option<CullingAction> {
        let materials = match self.materials().await {
            Ok(materials) => materials,
            Err(error) => {
                self.alert("Couldn't save keep or reject", &error.to_string());
                return None;
            }
        };
        let index = CatalogIndex::new(&materials);
        let current: Vec<Culling> = ids
            .iter()
            .filter(|id| index.entries.contains_key(id))
            .map(|id| index.culling(*id))
            .collect();
        let action = action.toggled(&current);
        let writes = self.locked().culling.apply(action, ids, &index)?;
        // A failed save only says so; the step stays in the undo history, as in `LibraryView.saveCulling`.
        if let Err(error) = self.store.set_culling(&writes).await {
            self.alert("Couldn't save keep or reject", &error.to_string());
        }
        self.notify();
        Some(action)
    }

    /// Takes back this session's last keep, reject or clear and returns the result it restored, which becomes the
    /// current card again; `None` with nothing to undo.
    pub async fn undo_culling(&self) -> Option<Uuid> {
        let writes = self.locked().culling.undo()?;
        let restored = writes.keys().map(|reference| reference.material_id).next();
        if let Err(error) = self.store.set_culling(&writes).await {
            self.alert("Couldn't save keep or reject", &error.to_string());
        }
        self.notify();
        restored
    }

    /// Makes the variant the face of its group's card.
    pub async fn make_primary(&self, id: Uuid) {
        if let Err(error) = self.store.make_variant_primary(id).await {
            self.alert("Couldn't make this variant primary", &error.to_string());
        }
    }

    /// Loads the first page of the account's history again, or the next one, caching each in the catalog. Pages
    /// that hold nothing the Library shows are skipped over. Does nothing without an account connection.
    pub async fn load_history(&self, refresh: bool) {
        let owner = self.owner();
        let (source, generation, mut request) = {
            let mut state = self.locked();
            let Some(source) = state.account.history.clone() else {
                return;
            };
            state.history.switch_account(owner.clone());
            let Some(request) = state.history.begin_load(refresh) else {
                return;
            };
            (source, state.account.generation, request)
        };
        self.notify();
        loop {
            let page = match source.history(request.cursor.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    if error == PortError::Provider(ProviderError::Unauthorized) {
                        self.connection.mark_needs_attention();
                    }
                    tracing::error!(%error, "can't load the history");
                    self.locked().history.fail(&error, &request);
                    self.notify();
                    return;
                }
            };
            if self.provider.generation() != generation {
                self.locked().history.cancel(&request);
                self.notify();
                return;
            }
            let page = match self.store.cache_history(page, owner.as_deref()).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::error!(%error, "can't keep the history in the catalog");
                    self.locked().history.fail_with(
                        UserMessage::new(
                            "Can't load your Higgsfield history",
                            "Try again in a moment.",
                            None,
                        ),
                        &request,
                    );
                    self.notify();
                    return;
                }
            };
            let next = {
                let mut state = self.locked();
                if state.history.receive(page, &request) {
                    state.history.begin_load(false)
                } else {
                    None
                }
            };
            self.notify();
            match next {
                Some(next) => request = next,
                None => return,
            }
        }
    }

    /// Downloads a result or image only on Higgsfield into the Library: a video through the engine's import, an
    /// image into the output folder's Images.
    pub async fn download(&self, id: Uuid) {
        let material = match self.material(id).await {
            Ok(material) => material,
            Err(error) => return self.alert("Can't download this video", &error.to_string()),
        };
        if !self.locked().downloading.insert(id) {
            return;
        }
        self.notify();
        let result = match material_row(&material) {
            Some(MediaRow::Video(item)) => self.import_video(&item).await,
            Some(MediaRow::RemoteImage(_)) => self.download_image(&material).await,
            _ => Ok(()),
        };
        self.locked().downloading.remove(&id);
        self.notify();
        if let Err(error) = result {
            tracing::error!(%error, "can't download a result");
            let title = if material.kind == MaterialKind::Image {
                "Can't download this image"
            } else {
                "Can't download this video"
            };
            self.alert(title, "Try again in a moment.");
        }
    }

    /// Downloads a result or image only on Higgsfield as `download` does, for an agent: what failed comes back instead
    /// of an alert. A result with a file here already is left as it is.
    pub async fn download_quietly(&self, material: &Material) -> Result<(), String> {
        match material_row(material) {
            Some(MediaRow::Video(item)) => self.import_video(&item).await,
            Some(MediaRow::RemoteImage(_)) => self.download_image(material).await,
            _ => Ok(()),
        }
    }

    async fn import_video(&self, item: &LibraryItem) -> Result<(), String> {
        let Some(history) = &item.history_item else {
            return Ok(());
        };
        let job = Job::importing(history, self.owner(), OffsetDateTime::now_utc());
        self.engine
            .import_finished(job, history.output_url.clone())
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn download_image(&self, material: &Material) -> Result<(), String> {
        let version = &material.current;
        let url = version.remote_url.clone().ok_or("The image has no link.")?;
        let generation = version.generation.clone().unwrap_or(MaterialGeneration {
            prompt: String::new(),
            model: String::new(),
            parameters: None,
            title: None,
        });
        let extension = url
            .path_segments()
            .and_then(|mut segments| segments.next_back().map(str::to_owned))
            .and_then(|name| {
                name.rsplit_once('.')
                    .map(|(_, extension)| extension.to_lowercase())
            })
            .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp"))
            .unwrap_or_else(|| "png".to_owned());
        let name = SavedImage::base_name(&generation.prompt, material.created_at, self.utc_offset);
        let file = OutputSubfolder::Images
            .path(&self.output_folder())
            .join(format!("{name}.{extension}"));
        self.downloader
            .download_to(&url, &file, MediaLinkKind::Image)
            .await
            .map_err(|error| error.to_string())?;
        let file_url =
            Url::from_file_path(&file).map_err(|()| "The image's path can't be used.".to_owned())?;
        let image = SavedImage {
            id: material.id,
            file_url,
            prompt: generation.prompt.clone(),
            model: generation.model.clone(),
            aspect_ratio: version.aspect_ratio.clone(),
            created_at: material.created_at,
            remote_url: Some(url),
            is_starred: material.is_starred,
            title: generation.title.clone(),
        };
        let remote: Option<MaterialRemote> = material.remote.clone();
        self.store
            .save_image(&image, remote, Some(generation), &[], material.variant_group_id)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// The files of `ids` that are here, for Show in file manager and Copy.
    pub async fn files(&self, ids: &[Uuid]) -> Vec<PathBuf> {
        let Ok(materials) = self.materials().await else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| materials.iter().find(|material| material.id == *id))
            .filter_map(|material| material.current.file_url.as_ref()?.to_file_path().ok())
            .filter(|path| path.exists())
            .collect()
    }

    /// The inputs of result `id` that have a copy here or on Higgsfield, and the name they are saved under.
    pub async fn inputs_of(&self, id: Uuid) -> Option<(Vec<InputFile>, String)> {
        let Ok(Some(MediaRow::Video(item))) = self.material(id).await.map(|material| material_row(&material))
        else {
            return None;
        };
        Some((InputFile::all(&item), item.source_name()))
    }

    /// Saves each input to its target: a copy of the file here while it is there, else Higgsfield's, downloaded. A
    /// file already at a target is replaced only once the new one is ready. Returns the files written; the ones
    /// that failed show as one alert (`InputExport.run`).
    pub async fn save_inputs(&self, targets: Vec<(InputFile, PathBuf)>) -> Vec<PathBuf> {
        let mut saved = Vec::new();
        let mut failed = Vec::new();
        for (input, target) in targets {
            let partial = target.with_file_name(format!(".{}.partial", Uuid::new_v4()));
            let copied = match &input.local {
                Some(path) if path.exists() => std::fs::copy(path, &partial)
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                _ => match &input.remote {
                    Some(url) => {
                        let kind = if input.role == InputRole::Source {
                            MediaLinkKind::Video
                        } else {
                            MediaLinkKind::Image
                        };
                        self.downloader
                            .download_to(url, &partial, kind)
                            .await
                            .map_err(|error| error.to_string())
                    }
                    None => Err("missing".to_owned()),
                },
            }
            .and_then(|()| std::fs::rename(&partial, &target).map_err(|error| error.to_string()));
            match copied {
                Ok(()) => saved.push(target),
                Err(error) => {
                    // The partial file may not exist; nothing to report when it doesn't.
                    let _ = std::fs::remove_file(&partial);
                    tracing::error!(%error, "can't save an input");
                    failed.push(input.title());
                }
            }
        }
        if !failed.is_empty() {
            self.alert(
                &super::inputs::failure_title(&failed),
                super::inputs::FAILURE_MESSAGE,
            );
        }
        saved
    }

    /// The chosen output folder, or the default one.
    fn output_folder(&self) -> PathBuf {
        self.preferences
            .get()
            .output_folder
            .map_or_else(|| self.layout.default_output_folder.clone(), PathBuf::from)
    }

    /// Every output folder results were saved to: the files Delete from Library may trash.
    fn output_folders(&self) -> Vec<PathBuf> {
        let preferences = self.preferences.get();
        let mut folders = vec![self.layout.default_output_folder.clone()];
        folders.extend(preferences.output_folder.map(PathBuf::from));
        folders.extend(
            preferences
                .previous_output_folders
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from),
        );
        folders
    }

    fn alert(&self, title: &str, message: &str) {
        self.connection.show_alert(UserMessage::new(title, message, None));
    }
}

/// Where the catalog's stars are written.
struct StoreStars(Arc<Store>);

#[async_trait]
impl StarStore for StoreStars {
    async fn set_starred(&self, is_starred: bool, id: Uuid) -> Result<(), String> {
        self.0
            .set_materials_starred(is_starred, &[id])
            .await
            .map_err(|error| error.to_string())
    }
}

/// What the grid says when no card shows for `request`.
fn grid_empty(request: &LibraryRequest, is_origin_empty: bool, images_total: usize) -> LibraryEmpty {
    if is_origin_empty {
        LibraryEmpty {
            title: "No results yet".to_owned(),
            detail: Some("Finished videos are saved to your output folder and appear here.".to_owned()),
            action: None,
        }
    } else if request.kind == LibraryKind::Image && images_total == 0 {
        images_empty(request.origin)
    } else {
        LibraryEmpty {
            title: "Nothing matches these filters".to_owned(),
            detail: None,
            action: request
                .filter
                .is_active()
                .then_some(LibraryEmptyAction::ClearFilters),
        }
    }
}

fn images_empty(origin: LibraryOrigin) -> LibraryEmpty {
    match origin {
        LibraryOrigin::Local => LibraryEmpty {
            title: "No images yet".to_owned(),
            detail: Some(
                "Images you make with Generate… in Create › References are saved here and in the Images folder of your output folder."
                    .to_owned(),
            ),
            action: Some(LibraryEmptyAction::GoToCreate),
        },
        LibraryOrigin::Higgsfield => LibraryEmpty {
            title: "No images on Higgsfield yet".to_owned(),
            detail: Some("Finished images from your account appear here once you make some.".to_owned()),
            action: None,
        },
    }
}

fn media_file(path: Option<&Path>, url_for: UrlFor<'_>) -> MediaFile {
    let exists = path.is_some_and(Path::exists);
    MediaFile {
        path: path.map(|path| path.to_string_lossy().into_owned()),
        url: path.filter(|_| exists).and_then(url_for),
        exists,
    }
}

fn link_view(link: &MaterialLink, url_for: UrlFor<'_>) -> LinkView {
    let preview_url = link.preview_url.as_ref().and_then(|url| {
        if url.scheme() == "file" {
            url.to_file_path()
                .ok()
                .filter(|path| path.exists())
                .and_then(|path| url_for(&path))
        } else {
            Some(url.to_string())
        }
    });
    LinkView {
        reference: link.reference,
        is_video: link.kind == MaterialKind::Video,
        role_title: link.role_title(),
        prompt: link.prompt.clone(),
        preview_url,
        availability: link.availability,
        note: link.availability_note().map(str::to_owned),
    }
}

/// "15 Oct 2026".
/// The note about a result's source, and the source to play beside it when it is on this computer.
fn source_parts(
    source: Option<&jutsu_core::SourceClip>,
    url_for: UrlFor<'_>,
) -> (String, Option<SourcePlayback>) {
    let note = match source {
        None => "No source video · made from the prompt".to_owned(),
        Some(clip) if clip.file_url.scheme() != "file" => "Source on Higgsfield".to_owned(),
        Some(clip) => source_file_name(&clip.file_url),
    };
    let playback = source
        .filter(|clip| clip.file_url.scheme() == "file")
        .and_then(|clip| {
            let path = clip.file_url.to_file_path().ok()?;
            Some(SourcePlayback {
                file: media_file(Some(&path), url_for),
                start: clip.range.lower(),
                end: clip.range.upper(),
            })
        });
    (note, playback)
}

/// The last part of a source file's path, "rooftop.mov".
fn source_file_name(file: &Url) -> String {
    file.to_file_path()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// "Upscaled from rooftop" on a result an operation made, linked to the result it was made from.
fn operation_source(item: &LibraryItem, items: &[LibraryItem]) -> Option<MadeFromLink> {
    MadeFromLink::new(
        item,
        item.job
            .parent_id
            .and_then(|parent| items.iter().find(|candidate| candidate.id() == parent)),
    )
}

pub(crate) fn date_text(date: OffsetDateTime, offset: UtcOffset) -> String {
    let local = date.to_offset(offset);
    let month = local.month().to_string();
    format!(
        "{} {} {}",
        local.day(),
        month.get(..3).unwrap_or(&month),
        local.year()
    )
}

mod delivery;
pub use delivery::{DeliveryExportScope, DeliveryFrameView, DeliveryRunSummary};
pub mod agent_exports;
mod editing;
mod enhance;
mod follow_ups;
mod image_compare;
pub use image_compare::ImagePixels;
mod spending;

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

/// An input's tile on a result's page: its title, the name it saves under, which tile it is and whether its file is
/// here.
fn input_view(input: &InputFile, base: &str) -> InputView {
    InputView {
        title: input.title(),
        file_name: input.file_name(base),
        is_source: input.role == InputRole::Source,
        reference: match input.role {
            InputRole::Reference(number) => u32::try_from(number).ok(),
            _ => None,
        },
        is_here: input.local.as_ref().is_some_and(|path| path.exists()),
    }
}
