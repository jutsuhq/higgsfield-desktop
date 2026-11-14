//! `library_search`, `library_mark` and `library_download`'s plan (`AgentLibrarySearch.swift`): the Library's own rows
//! and filters (`material_row`, `LibraryFilter`) over the catalog, read-only. Results with a file on this computer are
//! listed, since an agent works with paths, and the account's Higgsfield history too when asked for; rejected ones
//! only when asked for.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use jutsu_agent::{AgentLibraryItem, AgentLibraryKind, AgentVerdict, Timestamp};
use jutsu_core::{
    AgentMoney, Cost, Culling, HistoryImageItem, HistoryItem, Material, MaterialReference, Mode, PixelSize,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::budget::amount;
use super::job::{mode_name, page};
use super::wire_id;
use crate::library::CullingAction;
use crate::library::culling::{CatalogIndex, CullingHistory};
use crate::library::filter::{LibraryFilter, LibrarySort};
use crate::library::item::LibraryItem;
use crate::library::media::{LibraryKind, MediaRow, material_row};

/// What `library_search` asks for, before the wire's names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentLibraryQuery {
    pub text: String,
    pub kind: LibraryKind,
    /// `None` for both.
    pub starred: Option<bool>,
    /// `None` for every verdict but rejected, unless `include_rejected`.
    pub verdict: Option<Culling>,
    pub include_rejected: bool,
    /// A video model ID ("seedance25") or an image model's name; `None` for every model.
    pub model: Option<String>,
    /// A video mode; images have none, so a mode lists videos only.
    pub mode: Option<Mode>,
    pub from: Option<OffsetDateTime>,
    pub to: Option<OffsetDateTime>,
    pub limit: usize,
    pub cursor: Option<String>,
    /// Also the history rows From Higgsfield shows, with no file on this computer yet.
    pub include_higgsfield: bool,
}

/// One result as an agent sees it, before the wire's names.
#[allow(clippy::struct_excessive_bools, reason = "the wire's fields")]
#[derive(Debug, Clone, PartialEq)]
pub struct AgentLibraryHit {
    pub id: Uuid,
    pub is_video: bool,
    /// `None` for a result only on Higgsfield.
    pub file: Option<PathBuf>,
    /// A video model's ID, or an image model's name.
    pub model: String,
    /// The agent's mode name; `None` for an image.
    pub mode: Option<String>,
    pub aspect_ratio: Option<String>,
    pub duration: Option<f64>,
    /// The current version's frame, upright, when known.
    pub pixel_size: Option<PixelSize>,
    pub price: Option<Cost>,
    pub culling: Culling,
    pub is_starred: bool,
    pub created_at: OffsetDateTime,
    pub prompt: String,
    pub is_prompt_truncated: bool,
    pub version_id: Uuid,
    /// Which version is current, from 1.
    pub version: i64,
    pub version_count: i64,
    /// The job that made a video; `None` for an image and a video only on Higgsfield.
    pub job_id: Option<Uuid>,
    /// A result of the account's Higgsfield history with no file on this computer yet.
    pub is_on_higgsfield: bool,
}

/// Prompts are cut to this many characters in a search's answer.
pub const PROMPT_LENGTH: usize = jutsu_agent::LibrarySearchCommand::PROMPT_LENGTH;

/// The page of `materials` that `query` asks for, newest first, and the cursor of the next one.
pub fn search(materials: &[Material], query: &AgentLibraryQuery) -> (Vec<AgentLibraryHit>, Option<String>) {
    let index = CatalogIndex::new(materials);
    let by_id: HashMap<Uuid, &Material> = materials
        .iter()
        .rev()
        .map(|material| (material.id, material))
        .collect();
    let rows: Vec<MediaRow> = materials
        .iter()
        .filter_map(material_row)
        .filter(|row| query.include_higgsfield || is_local(row))
        .collect();
    let filter = LibraryFilter {
        query: query.text.clone(),
        is_starred_only: query.starred == Some(true),
        shows_rejected: query.include_rejected || query.verdict == Some(Culling::Rejected),
        sort: LibrarySort::Newest,
        ..LibraryFilter::default()
    };
    let hits: HashMap<Uuid, AgentLibraryHit> = filter
        .apply(&filter.culled(rows, &index), query.kind)
        .iter()
        .filter(|row| matches(row, index.culling(row.id()), query))
        .filter_map(|row| hit(row, by_id.get(&row.id())?, &index, Some(PROMPT_LENGTH)))
        .map(|hit| (hit.id, hit))
        .collect();
    let (ids, next) = page(
        hits.values().map(|hit| (hit.created_at, hit.id)).collect(),
        query.limit,
        query.cursor.as_deref(),
    );
    let mut hits = hits;
    (ids.iter().filter_map(|id| hits.remove(id)).collect(), next)
}

fn matches(row: &MediaRow, culling: Culling, query: &AgentLibraryQuery) -> bool {
    if query.starred == Some(false) && row.is_starred() {
        return false;
    }
    if query.verdict.is_some_and(|verdict| culling != verdict) {
        return false;
    }
    if query.from.is_some_and(|from| row.created_at() < from)
        || query.to.is_some_and(|to| row.created_at() > to)
    {
        return false;
    }
    match row {
        MediaRow::Video(item) => {
            let spec = item.spec();
            query.mode.is_none_or(|mode| spec.mode == mode)
                && query
                    .model
                    .as_deref()
                    .is_none_or(|model| spec.model.raw_value() == model)
        }
        MediaRow::LocalImage(image) => {
            query.mode.is_none() && query.model.as_deref().is_none_or(|model| image.model == model)
        }
        MediaRow::RemoteImage(image) => {
            query.mode.is_none() && query.model.as_deref().is_none_or(|model| image.model == model)
        }
    }
}

/// A row as an agent sees it; `prompt_length` cuts the prompt, `None` keeps it whole.
pub fn hit(
    row: &MediaRow,
    material: &Material,
    index: &CatalogIndex,
    prompt_length: Option<usize>,
) -> Option<AgentLibraryHit> {
    let version = &material.current;
    let versions = material.versions();
    let position = versions
        .iter()
        .position(|each| each.id == version.id)
        .unwrap_or(0)
        + 1;
    let generation_prompt = version
        .generation
        .as_ref()
        .map_or("", |generation| generation.prompt.as_str());
    let base = |is_video: bool, file: Option<PathBuf>, model: String, prompt: &str| {
        let (prompt, is_prompt_truncated) = truncated(prompt, prompt_length);
        AgentLibraryHit {
            id: material.id,
            is_video,
            file,
            model,
            mode: None,
            aspect_ratio: None,
            duration: None,
            pixel_size: version.pixel_size,
            price: None,
            culling: index.culling(material.id),
            is_starred: row.is_starred(),
            created_at: row.created_at(),
            prompt,
            is_prompt_truncated,
            version_id: version.id,
            version: count(position),
            version_count: count(versions.len()),
            job_id: None,
            is_on_higgsfield: false,
        }
    };
    match row {
        MediaRow::Video(item) => {
            let file = if item.history_item.is_none() {
                item.output_file()
            } else {
                None
            };
            if file.is_none() && item.history_item.is_none() {
                return None;
            }
            let spec = item.spec();
            let mut hit = base(true, file, spec.model.raw_value().to_owned(), &spec.prompt);
            hit.mode = Some(mode_name(spec.mode).to_owned());
            hit.aspect_ratio = spec
                .aspect_ratio
                .map(|ratio| ratio.raw_value().to_owned())
                .or_else(|| version.aspect_ratio.clone());
            hit.duration = Some(item.job.output_duration());
            hit.pixel_size = version
                .pixel_size
                .or_else(|| item.job.output_media.as_ref().map(|media| media.pixel_size));
            hit.price = (!item.is_from_higgsfield()).then(|| item.job.price().clone());
            // A history row's job is made up for the Library, not one `jobs` or `show` knows.
            hit.job_id = item.history_item.is_none().then_some(item.job.id);
            hit.is_on_higgsfield = item.history_item.is_some();
            Some(hit)
        }
        MediaRow::LocalImage(image) => {
            let mut hit = base(
                false,
                image.file_url.to_file_path().ok(),
                image.model.clone(),
                generation_prompt,
            );
            hit.aspect_ratio.clone_from(&image.aspect_ratio);
            Some(hit)
        }
        MediaRow::RemoteImage(image) => {
            let mut hit = base(false, None, image.model.clone(), generation_prompt);
            hit.aspect_ratio = image.aspect_ratio();
            hit.is_on_higgsfield = true;
            Some(hit)
        }
    }
}

/// The first `length` characters, and whether there were more; `None` keeps it whole.
pub fn truncated(prompt: &str, length: Option<usize>) -> (String, bool) {
    match length {
        Some(length) if prompt.chars().count() > length => (prompt.chars().take(length).collect(), true),
        _ => (prompt.to_owned(), false),
    }
}

/// It has a file on this computer: what an agent's Library commands act on.
pub fn is_local(row: &MediaRow) -> bool {
    match row {
        MediaRow::Video(item) => item.history_item.is_none() && item.output_file().is_some(),
        MediaRow::LocalImage(_) => true,
        MediaRow::RemoteImage(_) => false,
    }
}

/// The Library rows by their material's ID, the first of each kept.
pub fn rows_by_id(materials: &[Material], keep: impl Fn(&MediaRow) -> bool) -> HashMap<Uuid, MediaRow> {
    let mut rows = HashMap::new();
    for row in materials.iter().filter_map(material_row).filter(|row| keep(row)) {
        rows.entry(row.id()).or_insert(row);
    }
    rows
}

/// A hit with the wire's names.
pub fn agent_item(hit: &AgentLibraryHit) -> AgentLibraryItem {
    AgentLibraryItem {
        id: wire_id(hit.id),
        kind: if hit.is_video {
            AgentLibraryKind::Video
        } else {
            AgentLibraryKind::Image
        },
        file: hit.file.as_ref().map(|file| file.to_string_lossy().into_owned()),
        model: Some(hit.model.clone()),
        mode: hit.mode.clone(),
        aspect_ratio: hit.aspect_ratio.clone(),
        duration: hit.duration,
        width: hit.pixel_size.map(|size| size.width),
        height: hit.pixel_size.map(|size| size.height),
        price: hit.price.as_ref().and_then(AgentMoney::from_cost).map(amount),
        verdict: verdict(hit.culling),
        starred: hit.is_starred,
        created_at: Timestamp(hit.created_at),
        prompt: hit.prompt.clone(),
        prompt_truncated: hit.is_prompt_truncated,
        version_id: wire_id(hit.version_id),
        version: hit.version,
        version_count: hit.version_count,
        job_id: hit.job_id.map(wire_id),
        on_higgsfield: hit.is_on_higgsfield.then_some(true),
    }
}

pub fn verdict(culling: Culling) -> AgentVerdict {
    match culling {
        Culling::Unreviewed => AgentVerdict::Unreviewed,
        Culling::Kept => AgentVerdict::Kept,
        Culling::Rejected => AgentVerdict::Rejected,
    }
}

pub fn culling(verdict: AgentVerdict) -> Culling {
    match verdict {
        AgentVerdict::Unreviewed => Culling::Unreviewed,
        AgentVerdict::Kept => Culling::Kept,
        AgentVerdict::Rejected => Culling::Rejected,
    }
}

pub fn library_kind(kind: Option<AgentLibraryKind>) -> LibraryKind {
    match kind {
        None | Some(AgentLibraryKind::All) => LibraryKind::All,
        Some(AgentLibraryKind::Video) => LibraryKind::Video,
        Some(AgentLibraryKind::Image) => LibraryKind::Image,
    }
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// One result as `library_mark` leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub id: Uuid,
    pub culling: Culling,
    pub is_starred: bool,
}

/// `library_mark`: what Keep, Reject or Clear and the star change for the results named, worked out as the Library
/// does it (`CullingHistory::apply`, one star per result) before anything is written. Marking the same way again
/// changes nothing; nothing here hides a file or deletes one.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentLibraryMarks {
    /// The culling to write; empty when every result has the verdict already.
    pub culling: HashMap<MaterialReference, Culling>,
    /// The videos whose star changes.
    pub videos: Vec<LibraryItem>,
    /// The saved images whose star changes.
    pub images: HashSet<Uuid>,
    /// Each result as it is once the changes are written, in the order asked.
    pub results: Vec<Marked>,
    /// IDs that aren't results with a file on this computer: nothing is changed when there are any.
    pub missing: Vec<Uuid>,
}

impl AgentLibraryMarks {
    pub fn new(
        ids: &[Uuid],
        verdict: Option<CullingAction>,
        star: Option<bool>,
        materials: &[Material],
    ) -> Self {
        let index = CatalogIndex::new(materials);
        let rows = rows_by_id(materials, is_local);
        let missing: Vec<Uuid> = ids.iter().filter(|id| !rows.contains_key(id)).copied().collect();
        let culling = if missing.is_empty() {
            verdict
                .and_then(|action| CullingHistory::default().apply(action, ids, &index))
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        let (mut videos, mut images, mut results) = (Vec::new(), HashSet::new(), Vec::new());
        for id in ids {
            let Some(row) = rows.get(id) else {
                continue;
            };
            let is_starred = star.unwrap_or_else(|| row.is_starred());
            if missing.is_empty() && is_starred != row.is_starred() {
                match row {
                    MediaRow::Video(item) => videos.push(item.clone()),
                    MediaRow::LocalImage(_) => {
                        images.insert(*id);
                    }
                    MediaRow::RemoteImage(_) => {}
                }
            }
            results.push(Marked {
                id: *id,
                culling: verdict.map_or_else(|| index.culling(*id), CullingAction::result),
                is_starred,
            });
        }
        Self {
            culling,
            videos,
            images,
            results,
            missing,
        }
    }

    pub fn changes(&self) -> usize {
        self.culling.len() + self.videos.len() + self.images.len()
    }
}

/// `library_download`: what each ID asked for is, before anything is downloaded.
#[allow(
    clippy::large_enum_variant,
    reason = "a short-lived plan, one per ID of a call"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum AgentLibraryDownload {
    /// A video of the history, which the history import brings into the Library.
    Video(HistoryItem),
    /// An image of the history, which the image library saves.
    Image(HistoryImageItem),
    /// It has a file on this computer already.
    Local(PathBuf),
    /// No Library item has this ID.
    Missing,
}

impl AgentLibraryDownload {
    pub fn plan(ids: &[Uuid], materials: &[Material]) -> Vec<Self> {
        let rows = rows_by_id(materials, |_| true);
        ids.iter()
            .map(|id| match rows.get(id) {
                Some(MediaRow::Video(item)) => match (&item.history_item, item.output_file()) {
                    (Some(history), _) => Self::Video(history.clone()),
                    (None, Some(file)) => Self::Local(file),
                    (None, None) => Self::Missing,
                },
                Some(MediaRow::LocalImage(image)) => {
                    image.file_url.to_file_path().map_or(Self::Missing, Self::Local)
                }
                Some(MediaRow::RemoteImage(image)) => Self::Image(image.clone()),
                None => Self::Missing,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "library_search_tests.rs"]
mod tests;
