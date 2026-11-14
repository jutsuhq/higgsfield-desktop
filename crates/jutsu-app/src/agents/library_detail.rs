//! `library_get` (`AgentLibraryDetail.swift`): one Library result in full, read from the catalog as the Library's
//! details show it: the whole prompt, the files that went in, in the order its job sent them (`MaterialRelations` for
//! the Library items they are), and the settings it was made with. Rejected results too, and results only on
//! Higgsfield as far as the history says them, with no file; read-only.

use std::collections::HashMap;
use std::path::PathBuf;

use jutsu_agent::{
    AgentInputRole, AgentLibraryDetails, AgentLibraryInput, AgentLibrarySettings, AgentMadeFrom,
};
use jutsu_core::{
    ImageRequest, Job, Material, MaterialInputRole, MaterialReference, MaterialVersion, PostActionKind,
};
use url::Url;
use uuid::Uuid;

use super::library_search::{AgentLibraryHit, agent_item, hit, rows_by_id};
use super::video_order::audio_name;
use super::wire_id;
use crate::library::culling::CatalogIndex;
use crate::library::item::LibraryItem;
use crate::library::media::MediaRow;
use crate::library::relations::MaterialRelations;

/// One file that went into a result.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub role: AgentInputRole,
    /// 1-based among the job's images; `None` for the source and `edited_from`.
    pub position: Option<i64>,
    /// The file on this computer, when it is one.
    pub file: Option<PathBuf>,
    /// The Library item it is, when the Library lists it.
    pub item: Option<Uuid>,
    pub is_on_higgsfield: bool,
    /// The source's range, in seconds from the start of its file.
    pub range: Option<(f64, f64)>,
}

/// The version of the same item the current one was made from, and its place among the versions (1 for the
/// original).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadeFrom {
    pub reference: MaterialReference,
    pub version: i64,
    pub file: Option<PathBuf>,
}

/// One Library result in full.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentLibraryDetail {
    /// As `library_search` lists it, with the whole prompt.
    pub hit: AgentLibraryHit,
    pub inputs: Vec<Input>,
    pub settings: AgentLibrarySettings,
    pub made_from: Option<MadeFrom>,
}

/// The details of `ids`, in the order asked, and the IDs no Library item has.
pub fn details(ids: &[Uuid], materials: &[Material]) -> (Vec<AgentLibraryDetail>, Vec<Uuid>) {
    let index = CatalogIndex::new(materials);
    let mut by_id: HashMap<Uuid, &Material> = HashMap::new();
    for material in materials {
        by_id.entry(material.id).or_insert(material);
    }
    let rows = rows_by_id(materials, |_| true);
    let relations = MaterialRelations::new(materials, |_| true);
    let listed = |id: Option<Uuid>| {
        id.and_then(|id| by_id.get(&id))
            .filter(|material| !material.is_hidden)
            .map(|material| material.id)
    };
    let (mut found, mut missing) = (Vec::new(), Vec::new());
    for id in ids {
        let (Some(row), Some(material)) = (rows.get(id), by_id.get(id)) else {
            missing.push(*id);
            continue;
        };
        let Some(hit) = hit(row, material, &index, None) else {
            missing.push(*id);
            continue;
        };
        let (inputs, settings) = match row {
            MediaRow::Video(item) => (inputs(item, &material.current, &listed), settings(&item.job)),
            MediaRow::LocalImage(_) | MediaRow::RemoteImage(_) => (
                image_inputs(&material.current, &relations, &listed),
                image_settings(&material.current),
            ),
        };
        found.push(AgentLibraryDetail {
            hit,
            inputs,
            settings,
            made_from: made_from(material),
        });
    }
    (found, missing)
}

/// The earlier version of the same material its current version names as its parent (`MaterialVersions.parent`).
pub fn made_from(material: &Material) -> Option<MadeFrom> {
    let current = &material.current;
    let parent = current.inputs.iter().find(|input| {
        input.role == MaterialInputRole::Parent
            && input.source.material_id == material.id
            && input.source.version_id != current.id
    })?;
    let versions = material.versions();
    let index = versions
        .iter()
        .position(|version| version.id == parent.source.version_id)?;
    Some(MadeFrom {
        reference: MaterialReference::new(material.id, parent.source.version_id),
        version: count(index + 1),
        file: versions
            .get(index)
            .and_then(|version| version.file_url.as_ref())
            .and_then(local),
    })
}

/// The source, or the result an operation worked on, then the images in the job's order, then the speech.
fn inputs(
    item: &LibraryItem,
    version: &MaterialVersion,
    listed: &impl Fn(Option<Uuid>) -> Option<Uuid>,
) -> Vec<Input> {
    let spec = item.spec();
    let uploaded = item.job.inputs.as_ref().or(spec.remote_inputs.as_ref());
    let relation = |roles: &[MaterialInputRole], position: Option<i64>| {
        version
            .inputs
            .iter()
            .find(|input| {
                roles.contains(&input.role) && position.is_none_or(|position| input.position == position)
            })
            .map(|input| input.source.material_id)
    };
    let mut inputs = Vec::new();
    if let Some(source) = &spec.source {
        let is_operation = spec.operation.is_some();
        let made = if is_operation {
            relation(&[MaterialInputRole::Parent], None).or(item.job.parent_id)
        } else {
            relation(&[MaterialInputRole::SourceVideo], None)
        };
        inputs.push(Input {
            role: if is_operation {
                AgentInputRole::EditedFrom
            } else {
                AgentInputRole::Source
            },
            position: None,
            file: local(&source.file_url),
            item: listed(made),
            is_on_higgsfield: uploaded.is_some_and(|uploaded| uploaded.video.is_some())
                || spec
                    .operation
                    .as_ref()
                    .is_some_and(|operation| operation.source_job_id.is_some()),
            range: (!is_operation).then(|| (source.range.lower(), source.range.upper())),
        });
    }
    if spec.operation.is_none()
        && let Some(parent) = listed(relation(&[MaterialInputRole::Parent], None).or(item.job.parent_id))
    {
        inputs.push(Input {
            role: AgentInputRole::EditedFrom,
            position: None,
            file: None,
            item: Some(parent),
            is_on_higgsfield: false,
            range: None,
        });
    }
    let image_roles = [
        MaterialInputRole::StartFrame,
        MaterialInputRole::EndFrame,
        MaterialInputRole::Reference,
    ];
    for (offset, reference) in spec.references.iter().enumerate() {
        let role = match (spec.references_are_frames, offset) {
            (true, 0) => AgentInputRole::StartFrame,
            (true, _) => AgentInputRole::EndFrame,
            _ => AgentInputRole::Reference,
        };
        inputs.push(Input {
            role,
            position: Some(count(offset + 1)),
            file: local(&reference.file_url),
            item: listed(relation(&image_roles, Some(count(offset)))),
            is_on_higgsfield: uploaded.is_some_and(|uploaded| uploaded.images.len() > offset)
                || reference.server_media.is_some()
                || reference.file_url.scheme() != "file",
            range: None,
        });
    }
    if let Some(speech) = &spec.speech {
        inputs.push(Input {
            role: AgentInputRole::Speech,
            position: None,
            file: local(&speech.file_url),
            item: None,
            is_on_higgsfield: uploaded.is_some_and(|uploaded| uploaded.audio.is_some())
                || speech.file_url.scheme() != "file",
            range: None,
        });
    }
    inputs
}

/// An image's inputs, the images it edited, in the order they were sent.
fn image_inputs<F: Fn(&std::path::Path) -> bool>(
    version: &MaterialVersion,
    relations: &MaterialRelations<'_, F>,
    listed: &impl Fn(Option<Uuid>) -> Option<Uuid>,
) -> Vec<Input> {
    let mut references: Vec<_> = version
        .inputs
        .iter()
        .filter(|input| input.role == MaterialInputRole::Reference)
        .collect();
    references.sort_by_key(|input| input.position);
    references
        .into_iter()
        .map(|input| {
            let source = relations.version(input.source);
            Input {
                role: AgentInputRole::Reference,
                position: Some(input.position + 1),
                file: source.and_then(|source| source.file_url.as_ref()).and_then(local),
                item: listed(Some(input.source.material_id)),
                is_on_higgsfield: source.is_some_and(|source| source.remote_url.is_some()),
                range: None,
            }
        })
        .collect()
}

fn settings(job: &Job) -> AgentLibrarySettings {
    let spec = &job.spec;
    let made_here = spec.operation.is_none();
    AgentLibrarySettings {
        resolution: made_here.then(|| spec.resolution.raw_value().to_owned()),
        draft: made_here.then_some(spec.is_draft),
        audio: if made_here {
            audio_name(spec.audio).map(str::to_owned)
        } else {
            None
        },
        style: spec.preset_id.map(wire_id),
        length: spec.length,
        aspect_ratio: spec.aspect_ratio.map(|ratio| ratio.raw_value().to_owned()),
        paid_with_unlimited: Some(
            job.remote
                .as_ref()
                .map_or(spec.uses_unlimited, |remote| remote.paid_with_unlimited),
        ),
        operation: spec
            .operation
            .as_ref()
            .map(|operation| operation_name(operation.action.kind()).to_owned()),
        count: None,
    }
}

/// What the request said, when the catalog kept it.
fn image_settings(version: &MaterialVersion) -> AgentLibrarySettings {
    let request: Option<ImageRequest> = version
        .generation
        .as_ref()
        .and_then(|generation| generation.parameters.as_deref())
        .and_then(|parameters| serde_json::from_slice(parameters).ok());
    AgentLibrarySettings {
        aspect_ratio: request
            .as_ref()
            .map(|request| request.aspect_ratio.clone())
            .or_else(|| version.aspect_ratio.clone()),
        count: request.map(|request| request.count),
        ..AgentLibrarySettings::default()
    }
}

/// The agents' name of an Enhance operation, as `enhance` takes it.
pub fn operation_name(kind: PostActionKind) -> &'static str {
    match kind {
        PostActionKind::Upscale => "upscale",
        PostActionKind::Reframe => "reframe",
        PostActionKind::RemoveBackground => "remove_background",
        PostActionKind::Dub => "dub",
        PostActionKind::ChangeVoice => "change_voice",
    }
}

fn local(url: &Url) -> Option<PathBuf> {
    (url.scheme() == "file")
        .then(|| url.to_file_path().ok())
        .flatten()
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// One item in full as `library_get` answers it, with a pointer to `library_download` for one only on Higgsfield.
pub fn agent_details(detail: &AgentLibraryDetail) -> AgentLibraryDetails {
    AgentLibraryDetails {
        item: agent_item(&detail.hit),
        inputs: detail
            .inputs
            .iter()
            .map(|input| AgentLibraryInput {
                role: input.role,
                position: input.position,
                file: input
                    .file
                    .as_ref()
                    .map(|file| file.to_string_lossy().into_owned()),
                item: input.item.map(wire_id),
                on_higgsfield: input.is_on_higgsfield,
                start: input.range.map(|range| range.0),
                end: input.range.map(|range| range.1),
            })
            .collect(),
        settings: detail.settings.clone(),
        made_from: detail.made_from.as_ref().map(|made| AgentMadeFrom {
            item: wire_id(made.reference.material_id),
            version_id: wire_id(made.reference.version_id),
            version: made.version,
            file: made.file.as_ref().map(|file| file.to_string_lossy().into_owned()),
        }),
        next: detail
            .hit
            .is_on_higgsfield
            .then(|| AgentLibraryDetails::DOWNLOAD_NEXT.to_owned()),
    }
}

#[cfg(test)]
#[path = "library_detail_tests.rs"]
mod tests;
