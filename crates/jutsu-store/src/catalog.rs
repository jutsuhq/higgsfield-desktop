//! `MaterialRecord`: the single Library catalog for images and videos, its writes and its queries.

use std::collections::{BTreeMap, HashMap, HashSet};

use jutsu_core::{
    Culling, Job, JobState, Material, MaterialError, MaterialGeneration, MaterialInput, MaterialInputRole,
    MaterialKind, MaterialOrigin, MaterialReference, MaterialRemote, MaterialVersion, Subtitles,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::error::StoreError;
use crate::store::{Store, StoreChange, nanos, text, uuid};

const TABLE: &str = "materials";

/// The order of `Store::materials_matching`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MaterialSort {
    /// Newest first, as the Library shows them.
    #[default]
    NewestFirst,
    /// Oldest first.
    OldestFirst,
}

/// A page of the catalog. The defaults list every listed material, rejected ones included, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct MaterialQuery {
    /// Only videos or only images; `None` for both.
    pub kind: Option<MaterialKind>,
    /// Only starred ones.
    pub starred_only: bool,
    /// Also the ones taken off the Library.
    pub include_hidden: bool,
    /// Leaves out the ones whose current version is rejected.
    pub hide_rejected: bool,
    /// The order.
    pub sort: MaterialSort,
    /// At most this many; `None` for all.
    pub limit: Option<u32>,
    /// Skips this many first.
    pub offset: u32,
}

/// A stored material: decodes the JSON, checks it is the row's and that its version IDs are unique, and takes the
/// star from its column. A damaged entry is an error, never left out or replaced.
fn decode(row: &SqliteRow) -> Result<Material, StoreError> {
    let id = text(row, "id")?;
    let data = text(row, "material_data")?;
    let mut material: Material =
        serde_json::from_str(&data).map_err(|error| StoreError::damaged(TABLE, &id, error))?;
    let versions = material.versions();
    let unique: HashSet<Uuid> = versions.iter().map(|version| version.id).collect();
    if material.id.to_string() != id || unique.len() != versions.len() {
        return Err(MaterialError::InvalidRecord.into());
    }
    material.is_starred = row.try_get("is_starred")?;
    Ok(material)
}

pub(crate) async fn material_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<Material>, StoreError> {
    let row = sqlx::query("SELECT id, material_data, is_starred FROM materials WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(connection)
        .await?;
    row.as_ref().map(decode).transpose()
}

async fn material_by_remote_in(
    connection: &mut SqliteConnection,
    remote: &MaterialRemote,
) -> Result<Option<Material>, StoreError> {
    let row = sqlx::query("SELECT id, material_data, is_starred FROM materials WHERE remote_key = ? LIMIT 1")
        .bind(remote.key())
        .fetch_optional(connection)
        .await?;
    row.as_ref().map(decode).transpose()
}

async fn exists(connection: &mut SqliteConnection, id: Uuid) -> Result<bool, StoreError> {
    let found: Option<i64> = sqlx::query_scalar("SELECT 1 FROM materials WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(connection)
        .await?;
    Ok(found.is_some())
}

fn encode(material: &Material) -> Result<String, StoreError> {
    serde_json::to_string(material).map_err(StoreError::encode("material"))
}

async fn insert(connection: &mut SqliteConnection, material: &Material) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO materials (id, created_at, remote_key, file_path, variant_group_id, kind, is_hidden, culling, \
         material_data, is_starred) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(material.id.to_string())
    .bind(nanos(material.created_at))
    .bind(material.remote.as_ref().map(MaterialRemote::key))
    .bind(material.current.file_url.as_ref().map(Url::as_str))
    .bind(material.variant_group_id.map(|id| id.to_string()))
    .bind(material.kind.raw_value())
    .bind(material.is_hidden)
    .bind(material.current.culling.raw_value())
    .bind(encode(material)?)
    .bind(material.is_starred)
    .execute(&mut *connection)
    .await?;
    write_inputs(connection, material).await
}

/// Writes `material` over its record; the star column only when `update_star`.
pub(crate) async fn update(
    connection: &mut SqliteConnection,
    material: &Material,
    update_star: bool,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE materials SET created_at = ?, remote_key = ?, file_path = ?, variant_group_id = ?, kind = ?, \
         is_hidden = ?, culling = ?, material_data = ?, is_starred = CASE WHEN ? THEN ? ELSE is_starred END \
         WHERE id = ?",
    )
    .bind(nanos(material.created_at))
    .bind(material.remote.as_ref().map(MaterialRemote::key))
    .bind(material.current.file_url.as_ref().map(Url::as_str))
    .bind(material.variant_group_id.map(|id| id.to_string()))
    .bind(material.kind.raw_value())
    .bind(material.is_hidden)
    .bind(material.current.culling.raw_value())
    .bind(encode(material)?)
    .bind(update_star)
    .bind(material.is_starred)
    .bind(material.id.to_string())
    .execute(&mut *connection)
    .await?;
    write_inputs(connection, material).await
}

/// Copies every version's inputs into `material_inputs`, which "Used in" reads.
async fn write_inputs(connection: &mut SqliteConnection, material: &Material) -> Result<(), StoreError> {
    let id = material.id.to_string();
    sqlx::query("DELETE FROM material_inputs WHERE material_id = ?")
        .bind(&id)
        .execute(&mut *connection)
        .await?;
    for (index, version) in (0_i64..).zip(material.versions()) {
        for input in &version.inputs {
            sqlx::query(
                "INSERT INTO material_inputs (material_id, version_id, version_index, source_material_id, \
                 source_version_id, role, position) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(version.id.to_string())
            .bind(index)
            .bind(input.source.material_id.to_string())
            .bind(input.source.version_id.to_string())
            .bind(input.role.raw_value())
            .bind(input.position)
            .execute(&mut *connection)
            .await?;
        }
    }
    Ok(())
}

/// Inserts `proposed`, or merges it into the material of the same ID or remote identity. Refreshing a listing
/// keeps a downloaded file, the versions, exact request inputs, the culling, the subtitles and the star.
pub(crate) async fn upsert(
    connection: &mut SqliteConnection,
    proposed: Material,
) -> Result<Material, StoreError> {
    let mut value = proposed.clone();
    let mut known = material_in(&mut *connection, value.id).await?;
    if known.is_none()
        && let Some(remote) = &value.remote
    {
        known = material_by_remote_in(&mut *connection, remote).await?;
    }
    if let Some(known) = known {
        value = merged(value, &proposed, known)?;
    }
    if exists(&mut *connection, value.id).await? {
        update(connection, &value, false).await?;
    } else {
        insert(connection, &value).await?;
    }
    Ok(value)
}

/// `value`, a new listing of `known`, merged into it.
fn merged(mut value: Material, proposed: &Material, known: Material) -> Result<Material, StoreError> {
    let original = known.versions()[0].clone();
    let current = &mut value.current;
    // Refreshing a remote listing must not discard a downloaded file, versions or exact request inputs.
    if current.file_url.is_none() {
        current.file_url.clone_from(&original.file_url);
    }
    current.remote_url = current.remote_url.take().or_else(|| original.remote_url.clone());
    current.id = original.id;
    // Culling is the user's verdict; a refreshed listing knows nothing of it.
    current.culling = original.culling;
    current.subtitles.clone_from(&original.subtitles);
    let has_parameters = current
        .generation
        .as_ref()
        .is_some_and(|generation| generation.parameters.is_some());
    if !has_parameters && original.generation.is_some() {
        current.generation.clone_from(&original.generation);
    }
    if current.inputs.is_empty() {
        current.inputs.clone_from(&original.inputs);
    }
    current.pixel_size = current.pixel_size.or(original.pixel_size);
    current.aspect_ratio = current
        .aspect_ratio
        .take()
        .or_else(|| original.aspect_ratio.clone());
    if let Some(job) = current.video.as_mut()
        && let Some(local) = &original.video
        && job.output_file.is_none()
    {
        job.clone_from(local);
    }
    let mut updated = known.clone();
    updated.update_version(value.current)?;
    updated.remote = value.remote.or(known.remote);
    updated.variant_group_id = value.variant_group_id.or(known.variant_group_id);
    // Saving a file again lists a material the user removed earlier.
    if !proposed.is_hidden && proposed.current.file_url.is_some() {
        updated.is_hidden = false;
    }
    // Stars on a local result are user edits; a remote listing updates likes separately.
    updated.is_starred = known.is_starred;
    Ok(updated)
}

/// The catalog version of an input file: `reference` when the input names one, else the material whose current
/// file it is, else a new hidden material with nothing but the file.
pub(crate) async fn input_material(
    connection: &mut SqliteConnection,
    file: &Url,
    kind: MaterialKind,
    reference: Option<MaterialReference>,
) -> Result<MaterialReference, StoreError> {
    if let Some(reference) = reference {
        return Ok(reference);
    }
    let row = sqlx::query("SELECT id, material_data, is_starred FROM materials WHERE file_path = ? LIMIT 1")
        .bind(file.as_str())
        .fetch_optional(&mut *connection)
        .await?;
    if let Some(row) = row {
        return Ok(decode(&row)?.reference());
    }
    let now = OffsetDateTime::now_utc();
    let mut version = MaterialVersion::new(Uuid::new_v4(), now);
    if file.scheme() == "file" {
        version.file_url = Some(file.clone());
    } else {
        version.remote_url = Some(file.clone());
    }
    let mut material = Material::new(Uuid::new_v4(), kind, now, version);
    material.is_hidden = true;
    Ok(upsert(connection, material).await?.reference())
}

/// The versions a new job is made from, with their roles, pinned when it is first saved.
pub(crate) async fn inputs_of(
    connection: &mut SqliteConnection,
    job: &Job,
) -> Result<Vec<MaterialInput>, StoreError> {
    let mut inputs = Vec::new();
    if let Some(parent) = job.parent_id {
        inputs.push(parent_input(&mut *connection, parent).await?);
    }
    if let Some(source) = &job.spec.source {
        let reference = input_material(
            &mut *connection,
            &source.file_url,
            MaterialKind::Video,
            source.material,
        )
        .await?;
        inputs.push(input(reference, MaterialInputRole::SourceVideo, 0));
    }
    let uses_frames = job.spec.uses_frames();
    for (position, image) in (0_i64..).zip(&job.spec.references) {
        let role = match (uses_frames, position) {
            (true, 0) => MaterialInputRole::StartFrame,
            (true, _) => MaterialInputRole::EndFrame,
            (false, _) => MaterialInputRole::Reference,
        };
        let reference = input_material(
            &mut *connection,
            &image.file_url,
            MaterialKind::Image,
            image.material,
        )
        .await?;
        inputs.push(input(reference, role, position));
    }
    Ok(inputs)
}

/// The current version of the material `parent`, or its first version's identity when it is gone.
pub(crate) async fn parent_input(
    connection: &mut SqliteConnection,
    parent: Uuid,
) -> Result<MaterialInput, StoreError> {
    let reference = material_in(connection, parent).await?.map_or_else(
        || MaterialReference::new(parent, parent),
        |material| material.reference(),
    );
    Ok(input(reference, MaterialInputRole::Parent, 0))
}

pub(crate) const fn input(
    source: MaterialReference,
    role: MaterialInputRole,
    position: i64,
) -> MaterialInput {
    MaterialInput {
        source,
        role,
        position,
    }
}

/// Registers a done job's video in the catalog, with the inputs pinned at enqueue.
pub(crate) async fn register_video(
    connection: &mut SqliteConnection,
    job: &Job,
    inputs: Vec<MaterialInput>,
) -> Result<(), StoreError> {
    let (JobState::Done, Some(file)) = (&job.state, &job.output_file) else {
        return Ok(());
    };
    let parameters = if job.imported_at.is_none() {
        Some(serde_json::to_vec(&job.spec).map_err(StoreError::encode("job spec"))?)
    } else {
        None
    };
    let generation = MaterialGeneration {
        prompt: job.spec.prompt.clone(),
        model: job.spec.model.raw_value().to_owned(),
        parameters,
        title: None,
    };
    let remote = job.remote.as_ref().map(|remote| MaterialRemote {
        provider: job.provider_kind,
        owner: job.owner.clone(),
        id: remote.request_id.clone(),
        output: 0,
    });
    let mut version = MaterialVersion::new(job.id, job.finished_at.unwrap_or(job.created_at));
    version.file_url = Some(file.clone());
    version.generation = Some(generation);
    version.inputs = inputs;
    version.video = Some(Box::new(job.clone()));
    let mut material = Material::new(job.id, MaterialKind::Video, job.created_at, version);
    material.origin = if remote.is_some() {
        MaterialOrigin::Higgsfield
    } else {
        MaterialOrigin::Local
    };
    material.remote = remote;
    material.variant_group_id = job.variant_group_id;
    upsert(connection, material).await?;
    Ok(())
}

impl Store {
    /// The whole catalog, newest first. A damaged entry is reported, never omitted or overwritten with an empty
    /// library.
    pub async fn materials(&self) -> Result<Vec<Material>, StoreError> {
        let rows =
            sqlx::query("SELECT id, material_data, is_starred FROM materials ORDER BY created_at DESC")
                .fetch_all(self.pool())
                .await?;
        rows.iter().map(decode).collect()
    }

    /// The materials `query` selects, in its order. A damaged entry among them is reported.
    pub async fn materials_matching(&self, query: &MaterialQuery) -> Result<Vec<Material>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, material_data, is_starred FROM materials WHERE (?1 IS NULL OR kind = ?1) \
             AND (?2 = 0 OR is_starred = 1) AND (?3 = 1 OR is_hidden = 0) AND (?4 = 0 OR culling != ?5) \
             ORDER BY CASE WHEN ?8 THEN created_at END DESC, created_at ASC LIMIT ?6 OFFSET ?7",
        )
        .bind(query.kind.map(MaterialKind::raw_value))
        .bind(query.starred_only)
        .bind(query.include_hidden)
        .bind(query.hide_rejected)
        .bind(Culling::Rejected.raw_value())
        .bind(query.limit.map_or(-1, i64::from))
        .bind(i64::from(query.offset))
        .bind(query.sort == MaterialSort::NewestFirst)
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(decode).collect()
    }

    /// The material of `id`.
    pub async fn material(&self, id: Uuid) -> Result<Option<Material>, StoreError> {
        let mut connection = self.pool().acquire().await?;
        material_in(&mut connection, id).await
    }

    /// The material of the account's generation `remote`.
    pub async fn material_by_remote(&self, remote: &MaterialRemote) -> Result<Option<Material>, StoreError> {
        let mut connection = self.pool().acquire().await?;
        material_by_remote_in(&mut connection, remote).await
    }

    /// Writes `proposed` over the stored material of its ID, star included, or adds it as `upsert` does. A
    /// damaged stored entry fails the save and stays as it is.
    pub async fn save_material(&self, proposed: Material) -> Result<Material, StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let saved = if material_in(&mut *connection, proposed.id).await?.is_some() {
            update(connection, &proposed, true).await?;
            proposed
        } else {
            upsert(connection, proposed).await?
        };
        write.commit(&[StoreChange::Materials]).await?;
        Ok(saved)
    }

    /// Stars or unstars the materials of `ids`.
    pub async fn set_materials_starred(&self, starred: bool, ids: &[Uuid]) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        for id in ids {
            sqlx::query("UPDATE materials SET is_starred = ? WHERE id = ?")
                .bind(starred)
                .bind(id.to_string())
                .execute(&mut *write.transaction)
                .await?;
        }
        write.commit(&[StoreChange::Materials]).await
    }

    /// Takes the materials of `ids` off the Library (`Material::remove_from_library`): each entry stays as a source
    /// of other versions, but its local file is no longer listed.
    pub async fn remove_materials(&self, ids: &[Uuid]) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        for id in ids {
            if let Some(mut material) = material_in(&mut *connection, *id).await? {
                material.remove_from_library();
                update(&mut *connection, &material, true).await?;
            }
        }
        write.commit(&[StoreChange::Materials]).await
    }

    /// Sets the culling of each version; one whose material or version is gone is skipped. All or nothing.
    pub async fn set_culling(&self, changes: &HashMap<MaterialReference, Culling>) -> Result<(), StoreError> {
        let mut by_material: BTreeMap<Uuid, Vec<(Uuid, Culling)>> = BTreeMap::new();
        for (reference, culling) in changes {
            by_material
                .entry(reference.material_id)
                .or_default()
                .push((reference.version_id, *culling));
        }
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        for (material_id, versions) in by_material {
            let Some(mut material) = material_in(&mut *connection, material_id).await? else {
                continue;
            };
            for (version_id, culling) in versions {
                let Some(mut version) = find_version(&material, version_id) else {
                    continue;
                };
                version.culling = culling;
                material.update_version(version)?;
            }
            update(&mut *connection, &material, false).await?;
        }
        write.commit(&[StoreChange::Materials]).await
    }

    /// Sets or removes the subtitles of one exact version; `MaterialError::MissingMaterial` when the material or
    /// the version is gone.
    pub async fn set_subtitles(
        &self,
        subtitles: Option<Subtitles>,
        reference: MaterialReference,
    ) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let mut material = material_in(&mut *connection, reference.material_id)
            .await?
            .ok_or(MaterialError::MissingMaterial)?;
        let mut version =
            find_version(&material, reference.version_id).ok_or(MaterialError::MissingMaterial)?;
        version.subtitles = subtitles;
        material.update_version(version)?;
        update(connection, &material, false).await?;
        write.commit(&[StoreChange::Materials]).await
    }

    /// Makes the material the face of its variant group; every other member stops being it.
    /// `MaterialError::MissingMaterial` when it is gone or in no group.
    pub async fn make_variant_primary(&self, id: Uuid) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let group: Option<Option<String>> =
            sqlx::query_scalar("SELECT variant_group_id FROM materials WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&mut *connection)
                .await?;
        let group = group.flatten().ok_or(MaterialError::MissingMaterial)?;
        let rows =
            sqlx::query("SELECT id, material_data, is_starred FROM materials WHERE variant_group_id = ?")
                .bind(group)
                .fetch_all(&mut *connection)
                .await?;
        for row in &rows {
            let mut material = decode(row)?;
            let is_primary = material.id == id;
            if material.is_variant_primary == is_primary {
                continue;
            }
            material.is_variant_primary = is_primary;
            update(&mut *connection, &material, false).await?;
        }
        write.commit(&[StoreChange::Materials]).await
    }

    /// What the version `reference` was made from ("Made from"); nothing when it is gone.
    pub async fn made_from(&self, reference: MaterialReference) -> Result<Vec<MaterialInput>, StoreError> {
        Ok(self
            .material(reference.material_id)
            .await?
            .and_then(|material| find_version(&material, reference.version_id))
            .map(|version| version.inputs)
            .unwrap_or_default())
    }

    /// The versions made from `reference` ("Used in"): materials newest first, each one's versions oldest first.
    pub async fn used_in(&self, reference: MaterialReference) -> Result<Vec<MaterialReference>, StoreError> {
        let rows = sqlx::query(
            "SELECT i.material_id, i.version_id FROM material_inputs i JOIN materials m ON m.id = i.material_id \
             WHERE i.source_material_id = ? AND i.source_version_id = ? GROUP BY i.material_id, i.version_id \
             ORDER BY MAX(m.created_at) DESC, MIN(i.version_index) ASC",
        )
        .bind(reference.material_id.to_string())
        .bind(reference.version_id.to_string())
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                Ok(MaterialReference::new(
                    uuid("material_inputs", &text(row, "material_id")?)?,
                    uuid("material_inputs", &text(row, "version_id")?)?,
                ))
            })
            .collect()
    }
}

pub(crate) fn find_version(material: &Material, id: Uuid) -> Option<MaterialVersion> {
    material
        .versions()
        .into_iter()
        .find(|version| version.id == id)
        .cloned()
}
