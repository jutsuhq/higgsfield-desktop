//! Delivery sets (`LibraryViewDelivery`, `DeliverySetsSheet`, `DeliveryExportModel`): making a set from a Library
//! selection, editing it, the next round, deleting it, and exporting its files into a folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jutsu_core::{
    CropAspect, CropFrame, DeliveryFolder, DeliveryManifest, DeliveryPlan, DeliveryScope, DeliverySet,
    DeliverySetError, DeliveryTask, ExportNaming, Material, MaterialKind,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::editing::{taken_names, write_new};
use super::{LibraryError, LibraryService, UrlFor, media_file};
use crate::library::delivery::{
    self, DELETE_FAILED, DeliveryCounts, DeliveryItemView, DeliveryOutputOptions, DeliveryOutputView,
    DeliveryRunError, DeliverySetChange, DeliverySetRow, DeliverySetView, DeliveryTaskState, FROZEN_MESSAGE,
    MAKE_FAILED, SAVE_FAILED,
};
use crate::library::relations::MaterialRelations;
use crate::queue::MissingFfmpeg;

/// Hears each file's state in a run, by task ID.
pub type TaskReport = Arc<dyn Fn(&str, DeliveryTaskState) + Send + Sync>;

/// Edits are applied and saved one at a time, so a quick second edit never saves over the first.
pub(super) static SAVES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Which files Export…, Export N Pending or Retry N Failed writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryExportScope {
    All,
    Pending,
    Failed,
}

impl From<DeliveryExportScope> for DeliveryScope {
    fn from(scope: DeliveryExportScope) -> Self {
        match scope {
            DeliveryExportScope::All => Self::All,
            DeliveryExportScope::Pending => Self::Pending,
            DeliveryExportScope::Failed => Self::Failed,
        }
    }
}

/// The frame sheet of one item for one aspect.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryFrameView {
    /// "Frame 4:5".
    pub title: String,
    pub item_name: String,
    pub preview: crate::library::view::MediaFile,
    /// The picture's width over its height; `None` when unknown.
    pub picture_aspect: Option<f64>,
    /// The aspect's width over its height.
    pub aspect: f64,
    pub is_video: bool,
    pub frame: CropFrame,
    pub maximum_zoom: f64,
    /// What Center sets.
    pub centered: CropFrame,
}

/// What a run left to say: the alert after a run with failures or skips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryRunSummary {
    /// "12 done · 1 failed"; `None` when nothing failed or was skipped.
    pub problems: Option<String>,
}

fn save_message(error: DeliverySetError) -> String {
    match error {
        DeliverySetError::Frozen => FROZEN_MESSAGE.to_owned(),
        other => other.to_string(),
    }
}

impl LibraryService {
    pub(super) async fn delivery_set_model(&self, id: Uuid) -> Result<DeliverySet, LibraryError> {
        self.store
            .delivery_sets()
            .await
            .map_err(|error| LibraryError::Catalog(error.to_string()))?
            .into_iter()
            .find(|set| set.id == id)
            .ok_or_else(|| LibraryError::NotFound("Can't open this delivery set".to_owned()))
    }

    /// The sheet's list, newest first.
    pub async fn delivery_sets(&self) -> Result<Vec<DeliverySetRow>, LibraryError> {
        let mut sets = self
            .store
            .delivery_sets()
            .await
            .map_err(|error| LibraryError::Catalog(error.to_string()))?;
        sets.sort_by_key(|set| std::cmp::Reverse(set.created_at));
        Ok(sets
            .iter()
            .map(|set| DeliverySetRow {
                id: set.id.to_string(),
                name: set.name.clone(),
                status: delivery::status_text(set, self.utc_offset),
            })
            .collect())
    }

    /// One set's detail.
    pub async fn delivery_set(&self, id: Uuid, url_for: UrlFor<'_>) -> Result<DeliverySetView, LibraryError> {
        let set = self.delivery_set_model(id).await?;
        let materials = self.materials().await?;
        let relations = MaterialRelations::new(&materials, Path::exists);
        let running = {
            let state = self.locked();
            state
                .delivery_export
                .as_ref()
                .is_some_and(|(running, _)| *running == id)
                .then(|| state.delivery_states.clone())
        };
        let items = set
            .items()
            .iter()
            .map(|item| {
                let path = relations
                    .version(item.reference)
                    .and_then(|version| version.file_url.as_ref())
                    .and_then(|url| url.to_file_path().ok());
                let aspects = delivery::frame_aspects(&set.outputs, item.kind);
                let state = delivery::item_state(&set, item, running.as_ref());
                DeliveryItemView {
                    state_hint: state.as_ref().and_then(delivery::state_hint),
                    state,
                    material_id: item.reference.material_id.to_string(),
                    version_id: item.reference.version_id.to_string(),
                    name: if item.name.is_empty() {
                        "Untitled".to_owned()
                    } else {
                        item.name.clone()
                    },
                    is_video: item.kind == MaterialKind::Video,
                    preview: media_file(path.as_deref(), url_for),
                    frame_aspects: aspects
                        .iter()
                        .map(|aspect| delivery::DeliveryChoice {
                            value: aspect.key(),
                            title: aspect.title(),
                        })
                        .collect(),
                }
            })
            .collect();
        let pending = DeliveryPlan::tasks(&set, DeliveryScope::Pending, |_| false).len();
        let failed = DeliveryCounts::new(&set).failed;
        Ok(DeliverySetView {
            id: set.id.to_string(),
            name: set.name.clone(),
            status: delivery::status_text(&set, self.utc_offset),
            is_frozen: set.is_frozen(),
            lock_hint: if set.is_frozen() {
                "Versions locked after export"
            } else {
                "Versions lock on first export"
            }
            .to_owned(),
            items,
            outputs: set.outputs.iter().map(DeliveryOutputView::new).collect(),
            options: DeliveryOutputOptions::new(),
            note: set.note.clone(),
            review_link: set.review_link.clone(),
            counts: DeliveryCounts::new(&set).summary(),
            failed_count: i32::try_from(failed).unwrap_or(i32::MAX),
            pending_count: i32::try_from(pending).unwrap_or(i32::MAX),
            can_export: !set.items().is_empty() && !set.outputs.is_empty(),
            has_folder: set.folder().is_some(),
            is_exporting: self
                .locked()
                .delivery_export
                .as_ref()
                .is_some_and(|(running, _)| *running == id),
        })
    }

    /// New Delivery Set of the selected materials (`ids`), at their current versions; answers its ID.
    pub async fn new_delivery_set(&self, ids: &[Uuid]) -> Result<Uuid, String> {
        let materials = self.materials().await.map_err(|error| error.to_string())?;
        let picked: Vec<Material> = ids
            .iter()
            .filter_map(|id| materials.iter().find(|material| material.id == *id).cloned())
            .collect();
        let now = OffsetDateTime::now_utc();
        let set = DeliverySet::new(
            Uuid::new_v4(),
            delivery::default_name(now, self.utc_offset),
            now,
            DeliverySet::items_from(&picked),
            DeliverySet::default_outputs(),
        );
        if let Err(error) = self.store.save_delivery_set(&set).await {
            self.alert(MAKE_FAILED, &error.to_string());
            return Err(error.to_string());
        }
        Ok(set.id)
    }

    /// Applies `change` to the set `id` and saves it.
    pub async fn change_delivery_set(&self, id: Uuid, change: DeliverySetChange) -> Result<(), String> {
        let _save = SAVES.lock().await;
        let mut set = self
            .delivery_set_model(id)
            .await
            .map_err(|error| error.to_string())?;
        if let Err(error) = change.apply(&mut set) {
            let message = save_message(error);
            self.alert(SAVE_FAILED, &message);
            return Err(message);
        }
        self.store.save_delivery_set(&set).await.map_err(|error| {
            self.alert(SAVE_FAILED, &error.to_string());
            error.to_string()
        })
    }

    /// New Set from This One: the same materials at their newest versions; answers the new set's ID.
    pub async fn next_delivery_round(&self, id: Uuid) -> Result<Uuid, String> {
        let set = self
            .delivery_set_model(id)
            .await
            .map_err(|error| error.to_string())?;
        let materials = self.materials().await.map_err(|error| error.to_string())?;
        let next = set.next_round(
            delivery::next_round_name(&set.name),
            OffsetDateTime::now_utc(),
            |material_id| {
                materials
                    .iter()
                    .find(|material| material.id == material_id)
                    .map(Material::reference)
            },
        );
        if let Err(error) = self.store.save_delivery_set(&next).await {
            self.alert(MAKE_FAILED, &error.to_string());
            return Err(error.to_string());
        }
        Ok(next.id)
    }

    /// Delete…: the set only; exported files and the library's versions stay.
    pub async fn delete_delivery_set(&self, id: Uuid) -> Result<(), String> {
        self.store.delete_delivery_set(id).await.map_err(|error| {
            self.alert(DELETE_FAILED, &error.to_string());
            error.to_string()
        })
    }

    /// The frame sheet of the item `version_id` for the aspect `aspect`.
    pub async fn delivery_frame(
        &self,
        id: Uuid,
        version_id: Uuid,
        aspect: &str,
        url_for: UrlFor<'_>,
    ) -> Result<DeliveryFrameView, LibraryError> {
        let set = self.delivery_set_model(id).await?;
        let item = set
            .items()
            .iter()
            .find(|item| item.reference.version_id == version_id)
            .ok_or_else(|| LibraryError::NotFound("This item is no longer in the set.".to_owned()))?;
        let aspect = CropAspect::COMMON
            .into_iter()
            .find(|candidate| candidate.key() == aspect)
            .ok_or_else(|| LibraryError::NotFound("This frame is no longer offered.".to_owned()))?;
        let materials = self.materials().await?;
        let relations = MaterialRelations::new(&materials, Path::exists);
        let version = relations.version(item.reference);
        let path = version
            .and_then(|version| version.file_url.as_ref())
            .and_then(|url| url.to_file_path().ok());
        #[allow(clippy::cast_precision_loss, reason = "frame sizes are far below 2^52")]
        let picture_aspect = version
            .and_then(|version| version.pixel_size)
            .filter(|size| size.width > 0 && size.height > 0)
            .map(|size| size.width as f64 / size.height as f64);
        #[allow(clippy::cast_precision_loss, reason = "small aspect numbers")]
        let ratio = aspect.width as f64 / aspect.height as f64;
        Ok(DeliveryFrameView {
            title: format!("Frame {}", aspect.title()),
            item_name: if item.name.is_empty() {
                "Untitled".to_owned()
            } else {
                item.name.clone()
            },
            preview: media_file(path.as_deref(), url_for),
            picture_aspect,
            aspect: ratio,
            frame: delivery::item_frame(&set, item.reference, aspect),
            maximum_zoom: CropFrame::MAXIMUM_ZOOM,
            centered: CropFrame::CENTERED,
            is_video: item.kind == MaterialKind::Video,
        })
    }

    /// The folder Export N Pending and Retry N Failed write to without asking: the set's own, when it still exists.
    pub async fn delivery_folder(&self, id: Uuid) -> Option<PathBuf> {
        let set = self.delivery_set_model(id).await.ok()?;
        set.folder()
            .and_then(|folder| folder.url.to_file_path().ok())
            .filter(|path| path.is_dir())
    }

    /// Writes the files of the set `id` in `scope` into `folder`, telling `report` each file's state, then records
    /// the results and writes the manifest. One run at a time.
    pub async fn export_delivery_set(
        self: &Arc<Self>,
        id: Uuid,
        scope: DeliveryExportScope,
        folder: &Path,
        report: TaskReport,
    ) -> Result<DeliveryRunSummary, String> {
        let cancel = CancellationToken::new();
        {
            let mut state = self.locked();
            if state.delivery_export.is_some() {
                return Err("Another delivery is being exported.".to_owned());
            }
            state.delivery_export = Some((id, cancel.clone()));
            state.delivery_states.clear();
        }
        let service = Arc::downgrade(self);
        let report: TaskReport = Arc::new(move |task, task_state| {
            if let Some(service) = service.upgrade() {
                service
                    .locked()
                    .delivery_states
                    .insert(task.to_owned(), task_state.clone());
            }
            report(task, task_state);
        });
        let result = self.run_delivery(id, scope, folder, report, &cancel).await;
        self.locked().delivery_export = None;
        result
    }

    /// Cancel Export.
    pub fn cancel_delivery_export(&self) {
        if let Some((_, cancel)) = self.locked().delivery_export.as_ref() {
            cancel.cancel();
        }
    }

    async fn run_delivery(
        &self,
        id: Uuid,
        scope: DeliveryExportScope,
        folder: &Path,
        report: TaskReport,
        cancel: &CancellationToken,
    ) -> Result<DeliveryRunSummary, String> {
        let set = self
            .delivery_set_model(id)
            .await
            .map_err(|error| error.to_string())?;
        let taken = taken_names(folder);
        let tasks = DeliveryPlan::tasks(&set, scope.into(), |name| taken.contains(&name.to_lowercase()));
        let materials = self.materials().await.map_err(|error| error.to_string())?;
        let results = delivery::run(
            &tasks,
            |task| {
                let materials = &materials;
                async move { self.export_delivery_task(&task, materials, folder, cancel).await }
            },
            |task, state| report(task, state),
        )
        .await;
        let folder_url =
            Url::from_directory_path(folder).map_err(|()| "The folder can't be used.".to_owned())?;
        let own_folder = set
            .folder()
            .and_then(|own| own.url.to_file_path().ok())
            .is_some_and(|own| own == folder);
        let _save = SAVES.lock().await;
        let mut set = self
            .delivery_set_model(id)
            .await
            .map_err(|error| error.to_string())?;
        if !results.is_empty() {
            set.record(
                results,
                DeliveryFolder { url: folder_url },
                OffsetDateTime::now_utc(),
            );
            if let Err(error) = self.store.save_delivery_set(&set).await {
                self.alert(SAVE_FAILED, &error.to_string());
            }
            Self::write_manifest(&set, folder, own_folder);
        }
        let counts = DeliveryCounts::new(&set);
        Ok(DeliveryRunSummary {
            problems: (counts.failed > 0 || counts.skipped > 0)
                .then(|| counts.summary())
                .flatten(),
        })
    }

    /// The set's manifest beside its files; it replaces an earlier one only in the set's own folder. A failure is
    /// only logged.
    pub(super) fn write_manifest(set: &DeliverySet, folder: &Path, own_folder: bool) {
        let path = folder.join(ExportNaming::delivery_manifest_name(&set.name));
        let written = serde_json::to_string_pretty(&DeliveryManifest::new(set, OffsetDateTime::now_utc()))
            .map_err(|error| error.to_string())
            .and_then(|json| {
                if own_folder {
                    std::fs::write(&path, json).map_err(|error| error.to_string())
                } else {
                    write_new(&path, &json).map_err(|error| error.to_string())
                }
            });
        if let Err(error) = written {
            tracing::warn!(%error, "the delivery manifest wasn't written");
        }
    }

    pub(super) async fn export_delivery_task(
        &self,
        task: &DeliveryTask,
        materials: &[Material],
        folder: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), DeliveryRunError> {
        if cancel.is_cancelled() {
            return Err(DeliveryRunError::Canceled);
        }
        let relations = MaterialRelations::new(materials, Path::exists);
        let version = relations.version(task.item.reference);
        let source = version
            .and_then(|version| version.file_url.as_ref())
            .and_then(|url| url.to_file_path().ok())
            .filter(|path| path.exists())
            .ok_or(DeliveryRunError::SourceUnavailable)?;
        let tools = self
            .tools
            .as_ref()
            .ok_or_else(|| DeliveryRunError::Other(MissingFfmpeg::MESSAGE.to_owned()))?;
        let cues = version
            .and_then(|version| version.subtitles.as_ref())
            .map(|subtitles| subtitles.cues.as_slice());
        jutsu_media::delivery::export(tools, task, &source, cues, folder, None, cancel)
            .await
            .map(|_| ())
            .map_err(DeliveryRunError::from)
    }
}
