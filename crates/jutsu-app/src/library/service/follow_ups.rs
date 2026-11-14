//! The Library's follow-ups (`LibraryCompare`, `BatchComparison`, `ScoreVariants`, `RenderFinalSection`,
//! `RenderFinalsSheet`, `JoinWithOriginal`, `ShortPreviewExport`, `OutsideEditor`): Compare, batch cards and their
//! comparison, virality scores, finals, Join with Original, the short preview, Edit in… and preset names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jutsu_core::{
    Cost, Job, JobSpec, Material, MaterialInputRole, MaterialVersion, Mode, Resolution, StylePreset,
};
use jutsu_engine::EnqueueOptions;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::{LibraryError, LibraryService, UrlFor, media_file};
use crate::create::SubmitOutcome;
use crate::library::actions::{BulkActions, BulkEntry, ImageFile};
use crate::library::batch::LibraryBatch;
use crate::library::compare::{self, CompareLayout, ComparePair};
use crate::library::culling::CatalogIndex;
use crate::library::finals::{
    self, BatchFinals, FINALIZE_HELP, FINALIZE_TITLE, FinalSection, FinalsSheet, ResolutionOption,
};
use crate::library::item::LibraryItem;
use crate::library::media::{MediaRow, material_row};
use crate::library::outside_edit;
use crate::library::short_preview;
use crate::library::variants::VariantGroups;
use crate::library::view::{
    BatchCellView, BatchSummary, BatchView, CardMenu, CompareCandidate, CompareLayoutOption, CompareTrack,
    CompareView, FollowUp, LibraryCard, ScoreButton, ScoreOutcome, ScoreSheet,
};
use crate::library::virality::{
    self, UNKNOWN_PRICE_CONSENT, UNREADABLE, ViralityAvailability, ViralityEntry, ViralityPlan, ViralityView,
};
use crate::queue::QueueConfirmation;
use crate::queue::format;
use crate::queue::service::{budgeted, queued_notice, user_message};

/// How many results "Compare…" offers.
const CANDIDATE_LIMIT: usize = 12;
/// Render final's confirm button over the cap.
const RENDER_ANYWAY: &str = "Render Anyway";
/// A final already in the queue is not queued again: it would be charged twice.
const FINAL_QUEUED: &str = "Final queued · View in Queue";

impl LibraryService {
    // MARK: Selection

    /// What the selection bar can do with `ids`.
    pub async fn bulk(&self, ids: &[Uuid]) -> Result<BulkActions, LibraryError> {
        let materials = self.materials().await?;
        let starring = self.starring();
        let entries: Vec<BulkEntry> = ids
            .iter()
            .filter_map(|id| materials.iter().find(|material| material.id == *id))
            .filter_map(material_row)
            .map(|row| {
                let (needs_import, can_star, can_render_final, image) = match &row {
                    MediaRow::Video(item) => (
                        item.needs_import(),
                        starring.can_star(item),
                        crate::library::finals::can_render_final(item),
                        None,
                    ),
                    MediaRow::LocalImage(image) => {
                        let is_here = image.file_url.to_file_path().is_ok_and(|path| path.exists());
                        let file = if is_here {
                            ImageFile::Here
                        } else {
                            ImageFile::Missing
                        };
                        (false, true, false, Some(file))
                    }
                    MediaRow::RemoteImage(_) => (
                        true,
                        starring.likes.is_some(),
                        false,
                        Some(ImageFile::OnHiggsfield),
                    ),
                };
                BulkEntry {
                    id: row.id(),
                    needs_import,
                    can_star,
                    is_starred: row.is_starred(),
                    can_render_final,
                    has_file: matches!(&row, MediaRow::Video(item) if item.output_file().is_some()),
                    image,
                }
            })
            .collect();
        Ok(BulkActions::new(&entries))
    }

    /// The star button and key: stars `ids` unless every one is starred, else unstars them.
    pub async fn toggle_star(&self, ids: &[Uuid]) {
        let Ok(bulk) = self.bulk(ids).await else {
            return;
        };
        let starrable: Vec<Uuid> = bulk
            .starrable_ids
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect();
        if !starrable.is_empty() {
            self.set_starred(&starrable, bulk.stars_all).await;
        }
    }

    // MARK: Items

    /// Every video result of the catalog, with its preset's name.
    async fn video_items(&self) -> Result<Vec<LibraryItem>, LibraryError> {
        let materials = self.materials().await?;
        let mut rows: Vec<MediaRow> = materials.iter().filter_map(material_row).collect();
        self.name_presets(&mut rows).await;
        Ok(rows
            .into_iter()
            .filter_map(|row| match row {
                MediaRow::Video(item) => Some(item),
                _ => None,
            })
            .collect())
    }

    pub(super) async fn video_item(&self, id: Uuid) -> Result<LibraryItem, LibraryError> {
        self.video_items()
            .await?
            .into_iter()
            .find(|item| item.id() == id)
            .ok_or_else(|| LibraryError::NotFound("This result is no longer in the library.".to_owned()))
    }

    async fn items_of(&self, ids: &[Uuid]) -> Result<Vec<LibraryItem>, LibraryError> {
        let items = self.video_items().await?;
        Ok(ids
            .iter()
            .filter_map(|id| items.iter().find(|item| item.id() == *id).cloned())
            .collect())
    }

    // MARK: Preset names

    /// Gives each restyle result its preset's name, from the cached catalog and the preset shelves; an unknown
    /// preset starts one fetch of the provider's catalog, which caches it for the next look.
    pub(super) async fn name_presets(&self, rows: &mut [MediaRow]) {
        let names = self.preset_names().await;
        let mut unknown = false;
        for row in rows.iter_mut() {
            if let MediaRow::Video(item) = row
                && let Some(id) = item.spec().preset_id
            {
                item.preset_name = names.get(&id).cloned();
                unknown |= item.preset_name.is_none();
            }
        }
        if unknown {
            self.fetch_presets();
        }
    }

    /// The name of `spec`'s preset, when it is known.
    pub(super) async fn preset_name(&self, spec: &JobSpec) -> Option<String> {
        let id = spec.preset_id?;
        let name = self.preset_names().await.get(&id).cloned();
        if name.is_none() {
            self.fetch_presets();
        }
        name
    }

    fn catalog_file(&self) -> PathBuf {
        self.layout.preset_cache_folder().join("catalog.json")
    }

    /// Restyle preset names by ID: the cached catalog, then the favorite and recent presets.
    async fn preset_names(&self) -> HashMap<Uuid, String> {
        let mut presets: Vec<StylePreset> = tokio::fs::read(self.catalog_file())
            .await
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        let preferences = self.preferences.get();
        for shelf in [preferences.favorite_presets, preferences.recent_presets]
            .into_iter()
            .flatten()
        {
            presets.extend(serde_json::from_value::<Vec<StylePreset>>(shelf).unwrap_or_default());
        }
        let mut names = HashMap::new();
        for preset in presets {
            names.entry(preset.id).or_insert(preset.name);
        }
        names
    }

    /// Fetches the provider's presets once per provider and writes them to the cached catalog.
    fn fetch_presets(&self) {
        let generation = self.provider.generation();
        {
            let mut state = self.locked();
            if state.presets_fetched == Some(generation) {
                return;
            }
            state.presets_fetched = Some(generation);
        }
        let Some(provider) = self.provider.current() else {
            return;
        };
        let file = self.catalog_file();
        tokio::spawn(async move {
            let presets = match provider.presets().await {
                Ok(presets) if !presets.is_empty() => presets,
                Ok(_) => return,
                Err(error) => return tracing::warn!(%error, "can't read the presets for their names"),
            };
            let Ok(data) = serde_json::to_vec(&presets) else {
                return;
            };
            if let Some(folder) = file.parent()
                && let Err(error) = tokio::fs::create_dir_all(folder).await
            {
                return tracing::warn!(%error, "can't cache the presets");
            }
            if let Err(error) = tokio::fs::write(&file, data).await {
                tracing::warn!(%error, "can't cache the presets");
            }
        });
    }

    // MARK: Cards

    /// What the card's menu offers for `item`, whose material is `material`.
    pub(super) fn card_menu(
        &self,
        item: &LibraryItem,
        material: Option<&Material>,
        now: OffsetDateTime,
    ) -> CardMenu {
        let owner = self.owner();
        let local = item.output_file().filter(|path| path.exists());
        let follow_up = if item.history_item.is_some() {
            Some(FollowUp::EditInputs)
        } else if finals::finalize_spec(&item.job, owner.as_deref(), now).is_some() {
            Some(FollowUp::Finalize)
        } else if finals::can_render_final(item) {
            Some(FollowUp::RenderFinal)
        } else {
            None
        };
        CardMenu {
            can_compare: item.output_file().is_some() || item.needs_import(),
            can_copy_image: false,
            has_original: material.is_some_and(|material| original_version(material).is_some()),
            follow_up,
            can_rerun: item.history_item.is_none() && item.spec().operation.is_none(),
            can_edit_video: item.history_item.is_none() && local.is_some(),
            can_edit_in: local.is_some() && !item.needs_import(),
            can_join: can_join(item),
        }
    }

    /// The batches among the cards: the results of one batch, two or more of them on screen and none in a variant
    /// group, become one card at the first of them.
    pub(super) fn fold_batches(
        cards: Vec<LibraryCard>,
        rows: &[MediaRow],
        groups: &VariantGroups,
        now: OffsetDateTime,
    ) -> Vec<LibraryCard> {
        let shown: HashSet<String> = cards.iter().map(|card| card.id.clone()).collect();
        let mut members: HashMap<Uuid, Vec<LibraryItem>> = HashMap::new();
        for row in rows {
            if let MediaRow::Video(item) = row
                && let Some(batch) = item.job.batch_id
                && groups.group(item.id()).is_none()
                && shown.contains(&item.id().to_string())
            {
                members.entry(batch).or_default().push(item.clone());
            }
        }
        let batch_of: HashMap<String, Uuid> = members
            .iter()
            .filter(|(_, items)| items.len() > 1)
            .flat_map(|(batch, items)| items.iter().map(move |item| (item.id().to_string(), *batch)))
            .collect();
        let mut placed = HashSet::new();
        let mut folded = Vec::with_capacity(cards.len());
        for card in cards {
            let Some(batch) = batch_of.get(&card.id).copied() else {
                folded.push(card);
                continue;
            };
            if !placed.insert(batch) {
                continue;
            }
            let batch = LibraryBatch::new(batch, members.remove(&batch).unwrap_or_default());
            folded.push(batch_card(card, &batch, now));
        }
        folded
    }

    // MARK: Compare

    /// Results `ids` side by side, in that order.
    pub async fn compare(&self, ids: &[Uuid], url_for: UrlFor<'_>) -> Result<CompareView, LibraryError> {
        let items = self.items_of(ids).await?;
        let tracks: Vec<(LibraryItem, Vec<String>)> = match items.as_slice() {
            [left, right] => {
                let pair = ComparePair::new(left.clone(), right.clone()).ok_or_else(|| {
                    LibraryError::NotFound("Compare needs two results on this computer.".to_owned())
                })?;
                pair.tracks()
                    .into_iter()
                    .map(|(item, labels)| (item.clone(), labels))
                    .collect()
            }
            _ => items
                .iter()
                .filter(|item| item.output_file().is_some())
                .map(|item| (item.clone(), item.facts()))
                .collect(),
        };
        if tracks.len() < 2 {
            return Err(LibraryError::NotFound(
                "Compare needs two results on this computer.".to_owned(),
            ));
        }
        Ok(compare_view(
            tracks
                .iter()
                .map(|(item, labels)| track(item, labels.clone(), item.output_file().as_deref(), url_for))
                .collect(),
        ))
    }

    /// Compare with Original: the result beside the version it was made from.
    pub async fn compare_original(&self, id: Uuid, url_for: UrlFor<'_>) -> Result<CompareView, LibraryError> {
        let material = self.material(id).await?;
        let original = original_version(&material)
            .ok_or_else(|| LibraryError::NotFound("This result has no earlier version.".to_owned()))?;
        let item = match material_row(&material) {
            Some(MediaRow::Video(item)) => Some(item),
            _ => None,
        };
        let facts = item.as_ref().map(LibraryItem::facts).unwrap_or_default();
        let title = item.as_ref().map(LibraryItem::title).unwrap_or_default();
        let labelled = |label: &str| {
            let mut labels = vec![label.to_owned()];
            labels.extend(facts.clone());
            labels
        };
        let path =
            |version: &MaterialVersion| version.file_url.as_ref().and_then(|url| url.to_file_path().ok());
        Ok(compare_view(vec![
            CompareTrack {
                id: compare_id(original).to_string(),
                title: title.clone(),
                labels: labelled(compare::ORIGINAL_LABEL),
                file: media_file(path(original).as_deref(), url_for),
            },
            CompareTrack {
                id: material.id.to_string(),
                title,
                labels: labelled(compare::EDITED_LABEL),
                file: media_file(path(&material.current).as_deref(), url_for),
            },
        ]))
    }

    /// What "Compare…" offers for result `id`.
    pub async fn compare_candidates(&self, id: Uuid) -> Result<Vec<CompareCandidate>, LibraryError> {
        let items = self.video_items().await?;
        let item = items
            .iter()
            .find(|item| item.id() == id)
            .ok_or_else(|| LibraryError::NotFound("This result is no longer in the library.".to_owned()))?;
        Ok(compare::candidates(item, &items, CANDIDATE_LIMIT)
            .into_iter()
            .map(|other| CompareCandidate {
                id: other.id().to_string(),
                title: compare::menu_title(other),
            })
            .collect())
    }

    // MARK: Batch

    /// Batch `id` as a comparison.
    pub async fn batch(
        &self,
        id: Uuid,
        now: OffsetDateTime,
        url_for: UrlFor<'_>,
    ) -> Result<BatchView, LibraryError> {
        let materials = self.materials().await?;
        let index = CatalogIndex::new(&materials);
        let items = self.video_items().await?;
        let batch = LibraryBatch::find(id, &items)
            .ok_or_else(|| LibraryError::NotFound("This batch is no longer in the library.".to_owned()))?;
        let names: HashMap<Uuid, String> = batch
            .items
            .iter()
            .filter_map(|item| Some((item.spec().preset_id?, item.preset_name.clone()?)))
            .collect();
        let owner = self.owner();
        let plan = ViralityPlan::new(&batch.items, owner.as_deref());
        let has_scorer = self.locked().account.virality.is_some();
        let cells: Vec<BatchCellView> = batch
            .cells(&names)
            .into_iter()
            .map(|cell| BatchCellView {
                id: cell.item.id().to_string(),
                label: cell.label,
                file: media_file(cell.item.output_file().as_deref(), url_for),
                is_starred: cell.item.is_starred,
                culling: index.culling(cell.item.id()),
                virality: self.virality_view(&cell.item),
                can_render_final: finals::can_render_final(&cell.item),
            })
            .collect();
        let starred_finals = BatchFinals::new(&batch.items);
        Ok(BatchView {
            id: id.to_string(),
            title: batch.title(),
            subtitle: batch.subtitle(now),
            columns: count_i32(CompareLayout::columns(cells.len())),
            cells,
            unstarred_ids: batch
                .unstarred()
                .iter()
                .map(|item| item.id().to_string())
                .collect(),
            score: ScoreButton {
                availability: ViralityAvailability::of(has_scorer, true),
                title: plan.button_title().to_owned(),
                blocker: plan.blocker().map(str::to_owned),
                is_running: !self.locked().scoring.is_empty(),
            },
            can_render_finals: batch.items.iter().any(finals::can_render_final),
            has_starred_drafts: !starred_finals.is_empty(),
            finals_help: if starred_finals.is_empty() {
                "Star the drafts to render as finals".to_owned()
            } else {
                "Render the starred drafts as finals at one resolution".to_owned()
            },
            member_ids: batch.items.iter().map(|item| item.id().to_string()).collect(),
        })
    }

    // MARK: Virality

    /// A result's score as the window shows it.
    pub(super) fn virality_view(&self, item: &LibraryItem) -> ViralityView {
        let (is_scoring, failure) = {
            let state = self.locked();
            (
                state.scoring.contains(&item.id()),
                state.score_failures.get(&item.id()).cloned(),
            )
        };
        ViralityView::new(item, self.owner().as_deref(), is_scoring, failure)
    }

    /// The plan of scoring `ids`, its total, and the price of one result (`Cost::UNKNOWN_CREDITS` when it isn't
    /// known).
    async fn score_plan(&self, ids: &[Uuid]) -> Result<(ViralityPlan, Cost, Cost), LibraryError> {
        let items = self.items_of(ids).await?;
        let plan = ViralityPlan::new(&items, self.owner().as_deref());
        let scorer = self.locked().account.virality.clone();
        let each = match (scorer, plan.entries.first()) {
            (Some(scorer), Some(entry)) => scorer.virality_estimate(&entry.job_id).await.ok().flatten(),
            _ => None,
        };
        let total = plan.total(each.as_ref());
        Ok((plan, total, each.unwrap_or(Cost::UNKNOWN_CREDITS)))
    }

    /// The Score Variants sheet for `ids`.
    pub async fn score_sheet(&self, ids: &[Uuid]) -> Result<ScoreSheet, LibraryError> {
        let (plan, total, _) = self.score_plan(ids).await?;
        let is_unknown = total == Cost::UNKNOWN_CREDITS;
        let has_scorer = self.locked().account.virality.is_some();
        Ok(ScoreSheet {
            title: if plan.is_rescore {
                "Score Again"
            } else {
                "Score Variants"
            }
            .to_owned(),
            notice: plan.is_rescore.then(|| virality::RESCORE_NOTICE.to_owned()),
            price_text: plan.price_text(&total),
            button_title: if plan.is_rescore { "Score Again" } else { "Score" }.to_owned(),
            price_tag: (!is_unknown).then(|| ViralityPlan::price_tag(&total)),
            needs_consent: is_unknown,
            consent_text: UNKNOWN_PRICE_CONSENT.to_owned(),
            blocker: if has_scorer {
                plan.blocker().map(str::to_owned)
            } else {
                Some("Connect a Higgsfield account in Settings".to_owned())
            },
        })
    }

    /// Scores `ids` one at a time in the background, saving each score on its result (`VariantScorer.score`). Over
    /// the cap, or with an unknown price under one, it asks first unless `approved`; a result already being scored
    /// is left out, so a run never sends it twice.
    pub async fn start_scoring(
        self: &Arc<Self>,
        ids: &[Uuid],
        approved: bool,
    ) -> Result<ScoreOutcome, LibraryError> {
        let (plan, total, each) = self.score_plan(ids).await?;
        if let Some(reason) = plan.blocker() {
            return Ok(ScoreOutcome::Blocked {
                reason: reason.to_owned(),
            });
        }
        let Some(scorer) = self.locked().account.virality.clone() else {
            return Ok(ScoreOutcome::Blocked {
                reason: "Connect a Higgsfield account in Settings".to_owned(),
            });
        };
        if !approved && (each.amount() > rust_decimal::Decimal::ZERO || each.is_unknown_price()) {
            match self.cap_and_spend().await {
                Ok((committed, cap)) => {
                    if let Some(confirmation) =
                        ViralityPlan::cap_confirmation(&total, committed, cap.as_ref())
                    {
                        return Ok(ScoreOutcome::Confirm { confirmation });
                    }
                }
                Err(reason) => {
                    let mut state = self.locked();
                    for entry in &plan.entries {
                        state.score_failures.insert(entry.item_id, reason.clone());
                    }
                    drop(state);
                    self.notify();
                    return Ok(ScoreOutcome::Blocked { reason });
                }
            }
        }
        let entries: Vec<ViralityEntry> = {
            let mut state = self.locked();
            let state = &mut *state;
            take_unscored(plan.entries, &mut state.scoring, &mut state.score_failures)
        };
        if entries.is_empty() {
            return Ok(ScoreOutcome::Started);
        }
        self.notify();
        let service = Arc::clone(self);
        tokio::spawn(async move {
            for entry in entries {
                let failure = service.score_one(scorer.as_ref(), &entry, &each).await;
                {
                    let mut state = service.locked();
                    state.scoring.remove(&entry.item_id);
                    if let Some(failure) = failure {
                        state.score_failures.insert(entry.item_id, failure);
                    }
                }
                service.notify();
            }
        });
        Ok(ScoreOutcome::Started)
    }

    /// Scores one result. Its price goes into the image spend log as it is sent, so the cap and Spending count it;
    /// the answer settles it at what was charged, and a call refused before it ran takes it off again. Answers why
    /// no score was kept.
    pub(super) async fn score_one(
        &self,
        scorer: &dyn jutsu_core::ViralityScoring,
        entry: &ViralityEntry,
        each: &Cost,
    ) -> Option<String> {
        let charge = Uuid::new_v4();
        if let Err(error) = self.image_spend.add(jutsu_core::ImageCharge::new(
            charge,
            OffsetDateTime::now_utc(),
            each.clone(),
            jutsu_core::ProviderKind::Account,
            virality::MODEL_TITLE,
            1,
            Some(jutsu_core::ImageChargePurpose::ViralityScore),
        )) {
            return Some(crate::images::ImageSpendLog::not_sent(&error));
        }
        match scorer.score_virality(&entry.job_id).await {
            Ok(outcome) => {
                self.image_spend.settle(charge, outcome.charged);
                match outcome.score {
                    Some(score) => self.save_score(entry.item_id, score).await.err(),
                    None => Some(UNREADABLE.to_owned()),
                }
            }
            Err(error) => {
                if crate::images::generator::was_refused(&error) {
                    self.image_spend.remove(charge);
                } else {
                    self.image_spend.settle(charge, None);
                }
                Some(error.to_string())
            }
        }
    }

    /// Keeps `score` on the job of result `id`.
    async fn save_score(&self, id: Uuid, score: jutsu_core::ViralityScore) -> Result<(), String> {
        let mut material = self
            .materials()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|material| {
                material.id == id || material.current.video.as_ref().is_some_and(|job| job.id == id)
            })
            .ok_or("The result is no longer in the library.")?;
        let job = material.current.video.as_mut().ok_or("The result has no job.")?;
        job.virality = Some(score);
        self.store
            .save_material(material)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Today's committed spend, the image charges included, and the cap of the active connection
    /// (`submitDecision`). Without a cap nothing is read; under one, a spend that can't be read is the error that
    /// blocks the job, so nothing is queued at an unknown spend.
    pub(super) async fn cap_and_spend(
        &self,
    ) -> Result<(rust_decimal::Decimal, Option<jutsu_core::SpendCap>), String> {
        let kind = self.connection.state().kind();
        let Some(cap) = self.preferences.get().spend_cap(kind) else {
            return Ok((rust_decimal::Decimal::ZERO, None));
        };
        let committed = crate::settings::spending::committed_today(
            &self.store,
            &self.image_spend,
            cap.unit,
            OffsetDateTime::now_utc(),
            self.utc_offset,
        )
        .await
        .map_err(|_| jutsu_core::BudgetError::SpendingUnavailable.to_string())?;
        Ok((committed, Some(cap)))
    }

    // MARK: Finals

    pub(super) async fn estimate(&self, spec: &JobSpec) -> Option<Cost> {
        let provider = self.provider.current()?;
        provider.estimate(spec).await.ok()
    }

    /// The price a final is shown and capped at (`RenderFinalSection.finalPrice`): the bundled list price for an API
    /// key, the provider's estimate of an account's credits; `None` when neither is known.
    async fn final_cost(&self, spec: &JobSpec) -> Option<Cost> {
        match self.connection.state().kind()? {
            jutsu_core::ProviderKind::ApiKey => list_price(spec),
            jutsu_core::ProviderKind::Account => self.estimate(spec).await,
        }
    }

    /// Whether the final of `draft` at `spec` is already in the queue: rendering it again would charge twice.
    async fn queued_finals(&self) -> Vec<Job> {
        self.store.active_jobs().await.unwrap_or_default()
    }

    fn is_queued(active: &[Job], spec: &JobSpec, parent: Uuid) -> bool {
        active
            .iter()
            .any(|job| job.parent_id == Some(parent) && job.spec == *spec)
    }

    /// Save as Template…'s inputs (`LibraryItemModel.templateSpec`): a draft of a model with drafts is saved as its
    /// final at `resolution`, or at the one the Render final box opens on.
    pub async fn template_spec(
        &self,
        id: Uuid,
        resolution: Option<Resolution>,
        now: OffsetDateTime,
    ) -> Result<JobSpec, LibraryError> {
        let item = self.video_item(id).await?;
        let spec = item.spec();
        if !(spec.is_draft && spec.model.offers_draft()) {
            return Ok(spec.clone());
        }
        let resolutions = finals::final_resolutions(&item);
        let can_finalize = finals::finalize_spec(&item.job, self.owner().as_deref(), now).is_some();
        let selected = resolution
            .filter(|resolution| resolutions.contains(resolution))
            .unwrap_or_else(|| opening_resolution(can_finalize));
        Ok(finals::final_spec_at(&item, selected))
    }

    /// The "Render final" box of result `id` at `resolution`, or the one it opens on; `None` for a result that
    /// can't render a final.
    pub async fn final_section(
        &self,
        id: Uuid,
        resolution: Option<Resolution>,
        now: OffsetDateTime,
    ) -> Result<Option<FinalSection>, LibraryError> {
        let item = self.video_item(id).await?;
        if !finals::can_render_final(&item) {
            return Ok(None);
        }
        let owner = self.owner();
        let resolutions = finals::final_resolutions(&item);
        let finalize = finals::finalize_spec(&item.job, owner.as_deref(), now);
        let selected = resolution
            .filter(|resolution| resolutions.contains(resolution))
            .unwrap_or_else(|| opening_resolution(finalize.is_some()));
        let spec = finals::final_spec(&item, selected, owner.as_deref(), now);
        let is_finalize = spec.draft_job_id.is_some();
        let price = self.final_cost(&spec).await.map(|cost| format::cost_text(&cost));
        let options = finals::resolution_options(&resolutions, &item);
        let active = self.queued_finals().await;
        Ok(Some(FinalSection {
            detail: (options.len() == 1).then(|| options[0].title.clone()),
            options,
            selected,
            window: finals::finalize_deadline(&item.job)
                .map(|deadline| finals::finalize_window(deadline, now, self.utc_offset)),
            button_title: if is_finalize {
                FINALIZE_TITLE
            } else {
                "Render final"
            }
            .to_owned(),
            help: is_finalize.then(|| FINALIZE_HELP.to_owned()),
            price,
            is_queued: Self::is_queued(&active, &spec, item.id()),
            is_connected: self.provider.current().is_some(),
        }))
    }

    /// Render final of result `id` at `resolution`: Higgsfield's finalize at 1080p while it can run, else the
    /// draft's inputs again as a final. Over the cap it asks first unless `approved`.
    pub async fn render_final(&self, id: Uuid, resolution: Resolution, approved: bool) -> SubmitOutcome {
        let item = match self.video_item(id).await {
            Ok(item) => item,
            Err(error) => {
                return SubmitOutcome::Blocked {
                    reason: error.to_string(),
                };
            }
        };
        if !finals::can_render_final(&item) {
            return SubmitOutcome::Blocked {
                reason: "Only a draft in the library renders a final".to_owned(),
            };
        }
        let spec = finals::final_spec(
            &item,
            resolution,
            self.owner().as_deref(),
            OffsetDateTime::now_utc(),
        );
        if Self::is_queued(&self.queued_finals().await, &spec, item.id()) {
            return SubmitOutcome::Blocked {
                reason: FINAL_QUEUED.to_owned(),
            };
        }
        let finals = vec![finals::Final {
            spec,
            parent_id: item.id(),
        }];
        self.queue_finals(finals, approved).await
    }

    /// Render finals for starred: the sheet for results `ids` at `resolution`, or the one it opens on.
    pub async fn finals_sheet(
        &self,
        ids: &[Uuid],
        resolution: Option<Resolution>,
    ) -> Result<FinalsSheet, LibraryError> {
        let items = self.items_of(ids).await?;
        let batch = BatchFinals::new(&items);
        let resolutions = batch.resolutions();
        let selected = resolution
            .filter(|resolution| resolutions.contains(resolution))
            .unwrap_or_else(|| sheet_resolution(&resolutions));
        let active = self.queued_finals().await;
        let owner = self.owner();
        let finals = batch.finals(
            selected,
            owner.as_deref(),
            OffsetDateTime::now_utc(),
            |spec, parent| Self::is_queued(&active, spec, parent),
        );
        let mut prices = Vec::with_capacity(finals.len());
        for final_ in &finals {
            prices.push(self.final_cost(&final_.spec).await);
        }
        let options: Vec<ResolutionOption> = batch
            .drafts
            .first()
            .map(|draft| finals::resolution_options(&resolutions, draft))
            .unwrap_or_default();
        let left_out = batch.drafts.len() - finals.len();
        Ok(FinalsSheet {
            subtitle: LibraryBatch::new(Uuid::nil(), items).title(),
            detail: (options.len() == 1).then(|| options[0].title.clone()),
            options,
            selected,
            count: count_i32(finals.len()),
            button_title: BatchFinals::title(finals.len()),
            price: BatchFinals::total(&prices).map(|cost| format::cost_text(&cost)),
            queued_note: (left_out > 0).then(|| format!("Leaves out {left_out} already in the Queue.")),
            is_connected: self.provider.current().is_some(),
        })
    }

    /// Queues the finals of the starred drafts among `ids` at `resolution`, as one batch when more than one.
    pub async fn render_finals(&self, ids: &[Uuid], resolution: Resolution, approved: bool) -> SubmitOutcome {
        let items = match self.items_of(ids).await {
            Ok(items) => items,
            Err(error) => {
                return SubmitOutcome::Blocked {
                    reason: error.to_string(),
                };
            }
        };
        let active = self.queued_finals().await;
        let finals = BatchFinals::new(&items).finals(
            resolution,
            self.owner().as_deref(),
            OffsetDateTime::now_utc(),
            |spec, parent| Self::is_queued(&active, spec, parent),
        );
        if finals.is_empty() {
            return SubmitOutcome::Blocked {
                reason: "Star the drafts to render as finals".to_owned(),
            };
        }
        self.queue_finals(finals, approved).await
    }

    async fn queue_finals(&self, finals: Vec<finals::Final>, approved: bool) -> SubmitOutcome {
        if !approved {
            let mut prices = Vec::with_capacity(finals.len());
            for final_ in &finals {
                prices.push(self.final_cost(&final_.spec).await);
            }
            let total = BatchFinals::total(&prices);
            let (committed, cap) = match self.cap_and_spend().await {
                Ok(spend) => spend,
                Err(reason) => return SubmitOutcome::Blocked { reason },
            };
            if let Some(cap) = cap
                && let Some(confirmation) = budgeted(
                    i64::try_from(finals.len()).unwrap_or(i64::MAX),
                    total.as_ref(),
                    committed,
                    &cap,
                )
            {
                return SubmitOutcome::Confirm {
                    confirmation: QueueConfirmation {
                        confirm_title: RENDER_ANYWAY.to_owned(),
                        ..confirmation
                    },
                };
            }
        }
        let kind = self.connection.state().kind();
        let batch_id = (finals.len() > 1).then(Uuid::new_v4);
        let mut queued = Vec::with_capacity(finals.len());
        for final_ in finals {
            let options = EnqueueOptions {
                parent_id: Some(final_.parent_id),
                batch_id,
                over_cap_approved: approved,
                ..EnqueueOptions::default()
            };
            match self.engine.enqueue(final_.spec, options).await {
                Ok(job) => queued.push(job),
                Err(error) => {
                    if let Some(message) = user_message(&error, kind) {
                        self.connection.show_alert(message);
                    }
                    break;
                }
            }
        }
        match queued_notice(&queued) {
            Some(notice) => SubmitOutcome::Queued { notice },
            None => SubmitOutcome::Failed,
        }
    }

    // MARK: Files

    /// The name Join with Original… offers: "rooftop joined.mp4".
    pub async fn join_name(&self, id: Uuid) -> Result<String, LibraryError> {
        let item = self.video_item(id).await?;
        let stem = item
            .output_file()
            .and_then(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Video".to_owned());
        Ok(format!("{stem} joined.mp4"))
    }

    /// Join with Original: the source selection the Extend continued, then the new part, as one MP4 at `output`.
    /// A failure shows as the alert.
    pub async fn join_with_original(&self, id: Uuid, output: &Path) {
        const TITLE: &str = "Can't join the clips";
        let result = async {
            let item = self.video_item(id).await.map_err(|error| error.to_string())?;
            if !can_join(&item) {
                return Err(
                    "Only an Extend result with its source on this computer can be joined.".to_owned(),
                );
            }
            let tools = self.tools.as_ref().ok_or(MISSING_FFMPEG)?;
            let clip = item.spec().source.as_ref().ok_or("The source is missing.")?;
            let original = clip
                .file_url
                .to_file_path()
                .map_err(|()| "The source is missing.")?;
            let addition = item.output_file().ok_or("The result is missing.")?;
            let scratch = scratch_beside(output);
            jutsu_media::join::join(
                tools,
                &original,
                clip.range,
                &addition,
                &scratch,
                &CancellationToken::new(),
            )
            .await
            .map_err(|error| error.to_string())?;
            replace(&scratch, output)?;
            let url = Url::from_file_path(output).map_err(|()| "The file can't be saved there.")?;
            self.store
                .save_joined_video(&item.job, url)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        if let Err(message) = result {
            self.alert(TITLE, &message);
        }
    }

    /// The name Export Short Preview… offers: "result-preview.mp4".
    pub async fn preview_name(&self, id: Uuid) -> Result<String, LibraryError> {
        let path = self.local_file(id).await?;
        Ok(short_preview::file_name(&path))
    }

    async fn local_file(&self, id: Uuid) -> Result<PathBuf, LibraryError> {
        self.files(&[id])
            .await
            .into_iter()
            .next()
            .ok_or_else(|| LibraryError::NotFound("The file is no longer on this computer.".to_owned()))
    }

    /// Export Short Preview: the first 5 seconds at 480p without sound, under 2 MB, at `output`. A failure shows as
    /// the alert.
    pub async fn export_short_preview(&self, id: Uuid, output: &Path) {
        let result = async {
            let source = self.local_file(id).await.map_err(|error| error.to_string())?;
            let tools = self.tools.as_ref().ok_or(MISSING_FFMPEG)?;
            let info = jutsu_media::probe::probe(tools, &source)
                .await
                .map_err(|error| error.to_string())?;
            let range = short_preview::time_range(info.duration).ok_or("The video has no length.")?;
            let size = short_preview::render_size(info.pixel_size).ok_or("The video has no frame size.")?;
            let scratch = scratch_beside(output);
            jutsu_media::video::export_preview(
                tools,
                &source,
                range,
                size,
                short_preview::BYTE_LIMIT,
                &scratch,
                &CancellationToken::new(),
            )
            .await
            .map_err(|error| error.to_string())?;
            replace(&scratch, output)
        }
        .await;
        if let Err(message) = result {
            self.alert(short_preview::FAILED, &message);
        }
    }

    /// Edit in…: a working copy of result or image `id`'s current version, for another app to open.
    pub async fn edit_in(&self, id: Uuid) -> Option<PathBuf> {
        let result = async {
            let material = self.material(id).await.map_err(|error| error.to_string())?;
            let file = self.local_file(id).await.map_err(|error| error.to_string())?;
            outside_edit::working_copy(&file, material.reference(), &self.layout.edit_in_folder())
                .map_err(|error| error.to_string())
        }
        .await;
        result
            .inspect_err(|message| self.alert(outside_edit::EDIT_IN_FAILED, message))
            .ok()
    }

    /// Import as New Version: `file` copied next to the current version as "<name> edited", linked to the version
    /// its working copy was made from. A file of the item itself is no edit.
    pub async fn import_version(&self, id: Uuid, file: &Path) {
        let result = async {
            let material = self.material(id).await.map_err(|error| error.to_string())?;
            if outside_edit::is_own_file(file, &material) {
                return Ok(());
            }
            let root = self.layout.edit_in_folder();
            let parent = outside_edit::parent(file, &material, &root);
            let folder = material
                .current
                .file_url
                .as_ref()
                .and_then(|url| url.to_file_path().ok())
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| self.output_folder());
            let copy = outside_edit::copy_back(file, &folder).map_err(|error| error.to_string())?;
            let url = Url::from_file_path(&copy).map_err(|()| "The file can't be imported.")?;
            let added = self
                .store
                .add_edited_version(parent, url, None, false, OffsetDateTime::now_utc())
                .await;
            if let Err(error) = added {
                // The copy belongs to no version: take it away again.
                if let Err(removal) = std::fs::remove_file(&copy) {
                    tracing::warn!(%removal, "can't remove the imported copy");
                }
                return Err(error.to_string());
            }
            Ok(())
        }
        .await;
        if let Err(message) = result {
            self.alert(outside_edit::IMPORT_FAILED, &message);
        }
    }

    /// The image files of `ids` for Make Video… and Use as Reference, downloading the ones only on Higgsfield.
    pub async fn image_files(&self, ids: &[Uuid]) -> Vec<PathBuf> {
        let mut files = Vec::with_capacity(ids.len());
        for id in ids {
            if self.files(&[*id]).await.is_empty() {
                self.download(*id).await;
            }
            files.extend(self.files(&[*id]).await);
        }
        files
    }
}

const MISSING_FFMPEG: &str = "ffmpeg isn't installed. Install it and open the app again.";

/// The version `material`'s current one was made from, when it is another version of the same material whose file is
/// here (`MaterialVersions.original`): Compare with Original puts it on the left.
pub(super) fn original_version(material: &Material) -> Option<&MaterialVersion> {
    let input =
        material.current.inputs.iter().find(|input| {
            input.role == MaterialInputRole::Parent && input.source.material_id == material.id
        })?;
    material.previous_versions().iter().find(|version| {
        version.id == input.source.version_id
            && version
                .file_url
                .as_ref()
                .and_then(|url| url.to_file_path().ok())
                .is_some_and(|path| path.exists())
    })
}

/// The original's ID on a compare surface: its version's with every byte turned over, so it never equals the current
/// one's, as a material's first version has the material's own ID, which the item has too
/// (`MaterialVersions.compareID`).
pub(super) fn compare_id(version: &MaterialVersion) -> Uuid {
    let mut bytes = *version.id.as_bytes();
    for byte in &mut bytes {
        *byte ^= 0xFF;
    }
    Uuid::from_bytes(bytes)
}

/// Join with Original…: an Extend result with its file and a local source to put in front of it.
fn can_join(item: &LibraryItem) -> bool {
    item.spec().mode == Mode::Extend
        && item.output_file().is_some()
        && item
            .spec()
            .source
            .as_ref()
            .is_some_and(|clip| clip.file_url.scheme() == "file")
}

/// A file beside `output` to write into before it replaces `output`.
fn scratch_beside(output: &Path) -> PathBuf {
    let name = format!(".{}.mp4", Uuid::new_v4().simple());
    output
        .parent()
        .map_or_else(|| PathBuf::from(&name), |folder| folder.join(&name))
}

/// Moves `scratch` over `output`, which the save dialog already agreed to replace.
fn replace(scratch: &Path, output: &Path) -> Result<(), String> {
    std::fs::rename(scratch, output).map_err(|error| {
        let _ = std::fs::remove_file(scratch);
        error.to_string()
    })
}

fn count_i32(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

fn track(item: &LibraryItem, labels: Vec<String>, path: Option<&Path>, url_for: UrlFor<'_>) -> CompareTrack {
    CompareTrack {
        id: item.id().to_string(),
        title: item.title(),
        labels,
        file: media_file(path, url_for),
    }
}

fn compare_view(tracks: Vec<CompareTrack>) -> CompareView {
    CompareView {
        layouts: CompareLayout::available(tracks.len())
            .into_iter()
            .map(|layout| CompareLayoutOption {
                value: layout,
                title: layout.title().to_owned(),
            })
            .collect(),
        columns: count_i32(CompareLayout::columns(tracks.len())),
        tracks,
    }
}

/// The card of `batch` in place of its first result's `card`.
fn batch_card(mut card: LibraryCard, batch: &LibraryBatch, now: OffsetDateTime) -> LibraryCard {
    let count = batch.items.len();
    card.id = batch.id.to_string();
    card.title = batch.title();
    card.subtitle = batch.subtitle(now);
    card.accessibility_label = batch.accessibility_label(now);
    crate::connection::mode_title(batch.mode()).clone_into(&mut card.mode_tag);
    card.is_starred = batch.is_starred();
    card.can_star = false;
    card.variant_group = None;
    card.variant_count = None;
    card.request_id = None;
    card.virality = None;
    card.menu = CardMenu::default();
    card.batch = Some(BatchSummary {
        count: count_i32(count),
        tag: format!("Batch × {count}"),
        member_ids: batch.items.iter().map(|item| item.id().to_string()).collect(),
        unstarred_ids: batch
            .unstarred()
            .iter()
            .map(|item| item.id().to_string())
            .collect(),
        can_render_finals: batch.items.iter().any(finals::can_render_final),
    });
    card
}

/// The resolution the "Render final" box opens on (`LibraryDetail.finalResolution`): 720p, or 1080p for a draft
/// Higgsfield can finalize.
fn opening_resolution(can_finalize: bool) -> Resolution {
    if can_finalize {
        Resolution::P1080
    } else {
        Resolution::P720
    }
}

/// The resolution Render finals for starred opens on (`RenderFinalsSheet.init`): the first offered, else 720p.
fn sheet_resolution(resolutions: &[Resolution]) -> Resolution {
    resolutions.first().copied().unwrap_or(Resolution::P720)
}

/// The bundled list price of `spec`, an API key's final price.
fn list_price(spec: &JobSpec) -> Option<Cost> {
    static PRICING: std::sync::OnceLock<Option<jutsu_core::Pricing>> = std::sync::OnceLock::new();
    PRICING
        .get_or_init(|| jutsu_core::Pricing::bundled().ok())
        .as_ref()?
        .cost(spec)
        .ok()
}

/// The entries of a run that aren't being scored already (`VariantScorer.score`), now marked as scoring with their
/// last failure cleared.
fn take_unscored(
    entries: Vec<ViralityEntry>,
    scoring: &mut HashSet<Uuid>,
    failures: &mut HashMap<Uuid, String>,
) -> Vec<ViralityEntry> {
    let entries: Vec<ViralityEntry> = entries
        .into_iter()
        .filter(|entry| !scoring.contains(&entry.item_id))
        .collect();
    for entry in &entries {
        scoring.insert(entry.item_id);
        failures.remove(&entry.item_id);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::fixture::{ItemFixture, file};

    #[test]
    fn the_final_opens_at_720p_unless_higgsfield_can_finalize() {
        assert_eq!(opening_resolution(false), Resolution::P720);
        assert_eq!(opening_resolution(true), Resolution::P1080);
        assert_eq!(
            sheet_resolution(&[Resolution::P720, Resolution::P1080]),
            Resolution::P720
        );
        assert_eq!(sheet_resolution(&[]), Resolution::P720);
    }

    /// A final is in the queue only with exactly its spec: another draft's, or the same draft at another
    /// resolution or with other inputs, doesn't hold it back; Render final refuses one that is.
    #[test]
    fn only_the_exact_final_counts_as_queued() {
        let draft = ItemFixture::default().item();
        let spec = finals::final_spec_at(&draft, Resolution::P1080);
        let mut queued = draft.job.clone();
        queued.id = Uuid::new_v4();
        queued.parent_id = Some(draft.id());
        queued.spec = spec.clone();
        let active = vec![queued.clone()];
        assert!(LibraryService::is_queued(&active, &spec, draft.id()));
        assert!(!LibraryService::is_queued(&active, &spec, Uuid::new_v4()));
        let mut other = spec.clone();
        other.prompt = "Another prompt".to_owned();
        assert!(!LibraryService::is_queued(&active, &other, draft.id()));
    }

    #[test]
    fn an_api_keys_final_is_priced_from_the_list() {
        let draft = ItemFixture::default().item();
        let spec = finals::final_spec_at(&draft, Resolution::P1080);
        assert!(list_price(&spec).is_some());
    }

    #[test]
    fn a_result_being_scored_is_not_sent_again() {
        let (busy, free) = (Uuid::new_v4(), Uuid::new_v4());
        let mut scoring: HashSet<Uuid> = [busy].into();
        let mut failures: HashMap<Uuid, String> = [(free, "x".to_owned())].into();
        let entry = |item_id| ViralityEntry {
            item_id,
            job_id: "req".to_owned(),
        };
        let taken = take_unscored(vec![entry(busy), entry(free)], &mut scoring, &mut failures);
        assert_eq!(taken, vec![entry(free)]);
        assert!(scoring.contains(&free) && failures.is_empty());
        assert_eq!(
            take_unscored(vec![entry(free)], &mut scoring, &mut failures),
            Vec::<crate::library::virality::ViralityEntry>::new()
        );
    }

    #[test]
    fn join_needs_an_extend_with_a_local_source() {
        let mut item = ItemFixture::default().item();
        assert!(!can_join(&item));
        item.job.spec.mode = Mode::Extend;
        assert!(can_join(&item));
        item.job.output_file = None;
        assert!(!can_join(&item));
    }

    #[test]
    fn the_scratch_file_sits_beside_the_output() {
        let scratch = scratch_beside(Path::new("/tmp/out/clip joined.mp4"));
        assert_eq!(scratch.parent(), Some(Path::new("/tmp/out")));
        assert!(scratch.file_name().unwrap().to_string_lossy().starts_with('.'));
    }

    /// `MaterialVersionsTests`: the parent version of the same material, and only while its file is here.
    #[test]
    fn the_original_is_the_parent_version_of_the_same_material() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("rooftop.mp4");
        std::fs::write(&path, b"video").unwrap();
        let mut version = MaterialVersion::new(Uuid::new_v4(), crate::library::fixture::now());
        version.file_url = Some(crate::testing::file_url(&path));
        let mut material = Material::new(
            Uuid::new_v4(),
            jutsu_core::MaterialKind::Video,
            crate::library::fixture::now(),
            version,
        );
        assert!(original_version(&material).is_none());
        let original = material.reference();
        let mut edited = MaterialVersion::new(Uuid::new_v4(), crate::library::fixture::now());
        edited.file_url = Some(file("/tmp/rooftop edited.mp4"));
        edited.inputs = vec![jutsu_core::MaterialInput {
            source: original,
            role: MaterialInputRole::Parent,
            position: 0,
        }];
        material.add_version(edited).unwrap();
        assert_eq!(
            original_version(&material).map(|version| version.id),
            Some(original.version_id)
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            original_version(&material).is_none(),
            "an original whose file is gone isn't offered"
        );
    }

    #[test]
    fn a_batch_card_names_the_batch() {
        let batch_id = Uuid::new_v4();
        let mut first = ItemFixture::default().item();
        first.job.batch_id = Some(batch_id);
        let mut second = ItemFixture::default().item();
        second.job.batch_id = Some(batch_id);
        second.is_starred = true;
        let batch = LibraryBatch::new(batch_id, vec![first, second.clone()]);
        let card = LibraryCard {
            id: String::new(),
            kind: crate::library::view::CardKind::Video,
            title: String::new(),
            subtitle: String::new(),
            accessibility_label: String::new(),
            mode_tag: String::new(),
            is_draft: true,
            duration_text: None,
            aspect_text: None,
            prompt: String::new(),
            is_starred: false,
            can_star: true,
            culling: jutsu_core::Culling::Unreviewed,
            file_path: None,
            media_url: None,
            needs_import: false,
            is_downloading: false,
            is_unavailable: false,
            variant_group: None,
            variant_count: None,
            is_variant_primary: false,
            fit: crate::library::thumbnail::ThumbnailFit::new(None, crate::library::thumbnail::TILE_ASPECT),
            request_id: None,
            menu: CardMenu::default(),
            batch: None,
            virality: None,
        };
        let card = batch_card(card, &batch, crate::library::fixture::now());
        assert_eq!(card.id, batch_id.to_string());
        assert!(card.is_starred);
        let summary = card.batch.unwrap();
        assert_eq!(summary.tag, "Batch × 2");
        assert_eq!(summary.unstarred_ids.len(), 1);
        assert!(summary.can_render_finals);
    }
}
